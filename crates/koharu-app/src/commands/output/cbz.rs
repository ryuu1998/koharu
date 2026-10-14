use std::{
    io::{Seek, Write},
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Context as _, Result, ensure};
use futures::{StreamExt as _, TryStreamExt as _, stream};
use image::{ExtendedColorType, RgbImage, RgbaImage, codecs::jpeg::JpegEncoder};
use koharu_rasterizer::{RasterOptions, Rasterizer};
use koharu_renderer::Renderer;
use koharu_scene::Snapshot;
use zip::{ZipWriter, write::SimpleFileOptions};

use super::{page_stem, rasterize};

pub(crate) const DEFAULT_QUALITY: u8 = 95;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn export(
    snapshot: Snapshot,
    renderer: Renderer,
    rasterizer: Arc<Rasterizer>,
    destination: PathBuf,
    quality: u8,
    comic_info: Option<Vec<u8>>,
    overwrite: bool,
    on_page: Option<Arc<dyn Fn(usize) + Send + Sync>>,
) -> Result<()> {
    ensure!(
        (1..=100).contains(&quality),
        "JPEG quality must be between 1 and 100"
    );
    let pages = snapshot.pages().map(|page| page.id()).collect::<Vec<_>>();
    ensure!(!pages.is_empty(), "there are no pages to export");
    let width = pages.len().to_string().len().max(4);
    let parent = destination.parent().context("archive path has no parent")?;
    tokio::fs::create_dir_all(parent).await?;
    let temporary = tempfile::NamedTempFile::new_in(parent)?;
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<(String, Vec<u8>)>(2);
    let writer = tokio::task::spawn_blocking(move || -> Result<tempfile::NamedTempFile> {
        let mut archive = ZipWriter::new(temporary);
        while let Some((name, bytes)) = receiver.blocking_recv() {
            append(&mut archive, &name, &bytes)?;
            tracing::info!(target: "koharu_metrics", metric = "page_exported", format = "cbz");
        }
        if let Some(metadata) = comic_info {
            append(&mut archive, "ComicInfo.xml", &metadata)?;
        }
        let temporary = archive.finish()?;
        temporary.as_file().sync_all()?;
        Ok(temporary)
    });
    let images = stream::iter(pages)
        .enumerate()
        .map(|(index, page)| {
            let snapshot = snapshot.clone();
            let renderer = renderer.clone();
            let rasterizer = Arc::clone(&rasterizer);
            async move {
                let label = snapshot.page(page)?.page()?.label;
                let name = format!("{:0width$}_{}.jpg", index + 1, page_stem(&label));
                let frame = renderer.render(&snapshot, page).await?;
                let image = rasterize(rasterizer, &frame, RasterOptions::default())
                    .await?
                    .image;
                let bytes = tokio_rayon::spawn(move || encode(image, quality)).await?;
                Ok::<_, anyhow::Error>((name, bytes))
            }
        })
        .buffered(4);
    futures::pin_mut!(images);
    let prepared = async {
        let mut completed = 0;
        while let Some(image) = images.try_next().await? {
            sender
                .send(image)
                .await
                .map_err(|_| anyhow::anyhow!("archive writer stopped unexpectedly"))?;
            completed += 1;
            if let Some(on_page) = &on_page {
                on_page(completed);
            }
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    drop(sender);
    let temporary = writer
        .await
        .context("archive writer stopped unexpectedly")??;
    prepared?;
    if overwrite {
        temporary.persist(destination)
    } else {
        temporary.persist_noclobber(destination)
    }
    .map_err(|error| error.error)?;
    Ok(())
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
