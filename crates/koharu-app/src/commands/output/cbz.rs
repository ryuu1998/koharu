use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Seek, Write},
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Context as _, Result, ensure};
use futures::{Stream, StreamExt as _, stream::FuturesUnordered};
use image::{ExtendedColorType, RgbImage, RgbaImage, codecs::jpeg::JpegEncoder};
use koharu_pipeline::RunControl;
use koharu_rasterizer::{RasterOptions, Rasterizer};
use koharu_renderer::Renderer;
use koharu_scene::{EntityId, Snapshot};
use zip::{ZipWriter, write::SimpleFileOptions};

use super::{page_stem, rasterize};

pub(crate) const DEFAULT_QUALITY: u8 = 95;

pub(crate) struct PreparedCbz {
    directory: Arc<tempfile::TempDir>,
    pages: Vec<String>,
}

pub(crate) async fn prepare(
    snapshot: Snapshot,
    ready: impl Stream<Item = Result<(EntityId, Snapshot)>>,
    renderer: Renderer,
    rasterizer: Arc<Rasterizer>,
    quality: u8,
    control: RunControl,
    on_page: Option<Arc<dyn Fn(usize) + Send + Sync>>,
) -> Result<PreparedCbz> {
    ensure!(
        (1..=100).contains(&quality),
        "JPEG quality must be between 1 and 100"
    );
    let pages = snapshot.pages().map(|page| page.id()).collect::<Vec<_>>();
    ensure!(!pages.is_empty(), "there are no pages to export");
    let width = pages.len().to_string().len().max(4);
    let mut names = Vec::with_capacity(pages.len());
    let mut expected = BTreeMap::new();
    for (index, &page) in pages.iter().enumerate() {
        let label = snapshot.page(page)?.page()?.label;
        let name = format!("{:0width$}_{}.jpg", index + 1, page_stem(&label));
        names.push(name.clone());
        expected.insert(page, name);
    }
    let project = snapshot.project_id();
    let directory = Arc::new(tempfile::tempdir()?);
    let mut seen = BTreeSet::new();
    let mut running = FuturesUnordered::new();
    let mut ended = false;
    let mut failure = None;
    let mut completed = 0;
    let (cancel_pending, _) = tokio::sync::watch::channel(false);
    futures::pin_mut!(ready);
    while !ended || !running.is_empty() {
        let can_accept = !ended && running.len() < 4;
        let can_finish = !running.is_empty();
        tokio::select! {
            incoming = ready.next(), if can_accept => {
                match incoming {
                    Some(Ok((page, scene))) => {
                        let Some(name) = expected.get(&page)
                            .filter(|_| scene.project_id() == project && seen.insert(page)) else {
                            failure = Some(anyhow::anyhow!("render readiness is duplicated or outside the export scope"));
                            cancel_pending.send_replace(true);
                            ended = true;
                            continue;
                        };
                        let path = directory.path().join(name);
                        let renderer = renderer.clone();
                        let rasterizer = rasterizer.clone();
                        let control = control.clone();
                        let mut cancelled = cancel_pending.subscribe();
                        running.push(async move {
                            let activity = tokio::select! {
                                biased;
                                _ = cancelled.wait_for(|failed| *failed) => anyhow::bail!("rendering cancelled after an earlier error"),
                                activity = control.admit() => activity,
                            };
                            let Some(_activity) = activity else {
                                anyhow::bail!("rendering stopped");
                            };
                            let frame = renderer.render(&scene, page).await?;
                            let image = rasterize(rasterizer, &frame, RasterOptions::default()).await?.image;
                            let bytes = tokio_rayon::spawn(move || encode(image, quality)).await?;
                            tokio::fs::write(path, bytes).await?;
                            Ok::<_, anyhow::Error>(())
                        });
                    }
                    Some(Err(error)) => {
                        failure = Some(error);
                        cancel_pending.send_replace(true);
                        ended = true;
                    }
                    None => {
                        if seen.len() != pages.len() {
                            failure = Some(anyhow::anyhow!("render readiness ended after {} of {} pages", seen.len(), pages.len()));
                            cancel_pending.send_replace(true);
                        }
                        ended = true;
                    }
                }
            }
            result = running.next(), if can_finish => {
                match result {
                    Some(Ok(())) => {
                        completed += 1;
                        if let Some(on_page) = &on_page { on_page(completed); }
                    }
                    Some(Err(error)) => {
                        failure.get_or_insert(error);
                        cancel_pending.send_replace(true);
                        ended = true;
                    }
                    None => {}
                }
            }
        }
    }
    // Drain admitted raster work before returning an error. JPEGs are staged on
    // disk so out-of-order pages do not accumulate in RAM or publish a partial CBZ.
    if let Some(error) = failure {
        return Err(error);
    }
    ensure!(
        completed == pages.len(),
        "rendering ended after {completed} of {} pages",
        pages.len()
    );
    Ok(PreparedCbz {
        directory,
        pages: names,
    })
}

impl PreparedCbz {
    pub(crate) async fn write(
        self,
        destination: PathBuf,
        comic_info: Option<Vec<u8>>,
        overwrite: bool,
    ) -> Result<()> {
        let parent = destination
            .parent()
            .context("archive path has no parent")?
            .to_owned();
        tokio::fs::create_dir_all(&parent).await?;
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut archive = ZipWriter::new(tempfile::NamedTempFile::new_in(parent)?);
            for name in &self.pages {
                let mut file = std::fs::File::open(self.directory.path().join(name))
                    .context("a staged rendered page is missing")?;
                archive.start_file(
                    name,
                    SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
                )?;
                std::io::copy(&mut file, &mut archive)?;
            }
            if let Some(metadata) = comic_info {
                append(&mut archive, "ComicInfo.xml", &metadata)?;
            }
            let temporary = archive.finish()?;
            temporary.as_file().sync_all()?;
            if overwrite {
                temporary.persist(destination)
            } else {
                temporary.persist_noclobber(destination)
            }
            .map_err(|error| error.error)?;
            Ok(())
        })
        .await
        .context("archive writer stopped unexpectedly")?
    }
}

fn append<W: Write + Seek>(archive: &mut ZipWriter<W>, name: &str, bytes: &[u8]) -> Result<()> {
    archive.start_file(
        name,
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
    )?;
    archive.write_all(bytes)?;
    Ok(())
}

fn encode(image: RgbaImage, quality: u8) -> Result<Vec<u8>> {
    let image = composite_on_white(image);
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, quality).encode(
        image.as_raw(),
        image.width(),
        image.height(),
        ExtendedColorType::Rgb8,
    )?;
    Ok(bytes)
}

fn composite_on_white(image: RgbaImage) -> RgbImage {
    let (width, height) = image.dimensions();
    let mut bytes = Vec::with_capacity(width as usize * height as usize * 3);
    for pixel in image.as_raw().as_chunks::<4>().0 {
        let alpha = u16::from(pixel[3]);
        for channel in &pixel[..3] {
            bytes.push(((u16::from(*channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8);
        }
    }
    RgbImage::from_raw(width, height, bytes).expect("RGB length matches source dimensions")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[tokio::test]
    async fn successful_retranslation_replaces_only_the_selected_archive() {
        let directory = Arc::new(tempfile::tempdir().unwrap());
        std::fs::write(directory.path().join("page.jpg"), b"new page").unwrap();
        let output = tempfile::tempdir().unwrap();
        let target = output.path().join("chapter.cbz");
        let other = output.path().join("other.cbz");
        std::fs::write(&target, b"old translation").unwrap();
        std::fs::write(&other, b"other archive").unwrap();
        PreparedCbz {
            directory,
            pages: vec!["page.jpg".to_owned()],
        }
        .write(target.clone(), None, true)
        .await
        .unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(target).unwrap()).unwrap();
        let mut page = Vec::new();
        std::io::Read::read_to_end(&mut archive.by_index(0).unwrap(), &mut page).unwrap();
        assert_eq!(page, b"new page");
        assert_eq!(std::fs::read(other).unwrap(), b"other archive");
        assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 2);
    }

    #[tokio::test]
    async fn staged_pages_are_published_in_original_order() {
        let directory = Arc::new(tempfile::tempdir().unwrap());
        let names = vec!["0001_page.jpg".to_owned(), "0002_page.jpg".to_owned()];
        std::fs::write(directory.path().join(&names[1]), b"second").unwrap();
        std::fs::write(directory.path().join(&names[0]), b"first").unwrap();
        let output = tempfile::tempdir().unwrap();
        let path = output.path().join("chapter.cbz");
        PreparedCbz {
            directory,
            pages: names,
        }
        .write(path.clone(), Some(b"<ComicInfo />".to_vec()), false)
        .await
        .unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        assert_eq!(
            (0..archive.len())
                .map(|index| { archive.by_index(index).unwrap().name().to_owned() })
                .collect::<Vec<_>>(),
            ["0001_page.jpg", "0002_page.jpg", "ComicInfo.xml"]
        );
        let mut first = Vec::new();
        std::io::Read::read_to_end(&mut archive.by_index(0).unwrap(), &mut first).unwrap();
        assert_eq!(first, b"first");
    }

    #[tokio::test]
    async fn missing_staged_page_does_not_replace_an_existing_archive() {
        let directory = Arc::new(tempfile::tempdir().unwrap());
        let output = tempfile::tempdir().unwrap();
        let path = output.path().join("chapter.cbz");
        std::fs::write(&path, b"existing").unwrap();
        let result = PreparedCbz {
            directory,
            pages: vec!["missing.jpg".to_owned()],
        }
        .write(path.clone(), None, true)
        .await;
        assert!(result.is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"existing");
        assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 1);
    }

    #[test]
    fn flattens_transparent_pixels_over_white_before_jpeg_encoding() {
        let image = RgbaImage::from_raw(3, 1, vec![20, 30, 40, 0, 10, 20, 30, 255, 255, 0, 0, 128])
            .unwrap();
        assert_eq!(
            composite_on_white(image).into_raw(),
            [255, 255, 255, 10, 20, 30, 255, 127, 127]
        );
        let bytes = encode(
            RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 0])),
            DEFAULT_QUALITY,
        )
        .unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (1, 1));
        assert!(decoded.as_raw().iter().all(|channel| *channel >= 253));
    }

    #[test]
    fn writes_ordered_stored_entries_and_comic_metadata() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        append(&mut writer, "0001_page.jpg", b"first").unwrap();
        append(&mut writer, "0002_page.jpg", b"second").unwrap();
        append(&mut writer, "ComicInfo.xml", b"<ComicInfo />").unwrap();
        let bytes = writer.finish().unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes.into_inner())).unwrap();
        assert_eq!(
            (0..archive.len())
                .map(|index| archive.by_index(index).unwrap().name().to_owned())
                .collect::<Vec<_>>(),
            ["0001_page.jpg", "0002_page.jpg", "ComicInfo.xml"]
        );
        assert_eq!(
            archive.by_index(0).unwrap().compression(),
            zip::CompressionMethod::Stored
        );
        assert_eq!(archive.by_name("ComicInfo.xml").unwrap().size(), 13);
    }
}
