use std::{
    io::{Read as _, Write as _},
    path::Path,
    sync::Arc,
};

use anyhow::{Context as _, Result, ensure};
use image::{ColorType, ImageEncoder as _, codecs::jpeg::JpegEncoder};
use koharu_renderer::Renderer;
use koharu_scene::{Edit, Hash, Op, Session};
use parking_lot::Mutex;
use tempfile::TempDir;

use super::import;

/// Each chapter owns its own temporary project and never alters the open editor project.
pub(super) struct Chapter {
    pub session: Arc<Mutex<Session>>,
    names: Vec<String>,
    comic_info: Option<Vec<u8>>,
    _directory: TempDir,
}

impl Chapter {
    pub fn open(path: &Path) -> Result<Self> {
        let pages = import::extract_zip_named(path)?;
        let names = pages.iter().map(|(name, _)| name.clone()).collect();
        let comic_info = read_comic_info(path)?;
        let directory = tempfile::tempdir().context("create temporary chapter project")?;
        let mut session = Session::create(directory.path().join("chapter.khr"))?;
        let mut ops = Vec::with_capacity(pages.len() * 2);
        for (index, (_, bytes)) in pages.into_iter().enumerate() {
            let background = Some(Hash::of(&bytes));
            ops.push(Op::AddFile { bytes });
            ops.push(Op::AddPage {
                id: None,
                index,
                size: None,
                background,
            });
        }
        session.apply(Edit {
            label: "batch import".to_owned(),
            ops,
        })?;
        Ok(Self {
            session: Arc::new(Mutex::new(session)),
            names,
            comic_info,
            _directory: directory,
        })
    }

    /// Render one finished page to disk while the pipeline continues.
    pub fn render_page(
        &self,
        renderer: &Renderer,
        page: koharu_scene::PageId,
        index: usize,
        quality: u8,
        stage: &Path,
    ) -> Result<()> {
        ensure!(
            (1..=100).contains(&quality),
            "JPEG quality must be between 1 and 100"
        );
        let frame = renderer.render(&self.session.lock(), page)?;
        let rgba = frame.rasterize()?.image;
        let mut background = image::RgbaImage::from_pixel(
            rgba.width(),
            rgba.height(),
            image::Rgba([255, 255, 255, 255]),
        );
        image::imageops::overlay(&mut background, &rgba, 0, 0);
        let rgb = image::DynamicImage::ImageRgba8(background).into_rgb8();
        let file = std::fs::File::create(stage.join(format!("{index}.jpg")))?;
        JpegEncoder::new_with_quality(file, quality).write_image(
            rgb.as_raw(),
            rgba.width(),
            rgba.height(),
            ColorType::Rgb8.into(),
        )?;
        Ok(())
    }

    /// Publish only after every rendered page has been staged successfully.
    pub fn publish(
        &self,
        stage: &Path,
        destination: &Path,
        overwrite: bool,
        ready: impl Fn() -> Result<()>,
        publish: impl FnOnce(tempfile::NamedTempFile, &Path, bool) -> Result<()>,
    ) -> Result<()> {
        let parent = destination
            .parent()
            .context("output archive has no parent")?;
        std::fs::create_dir_all(parent)?;
        let mut zip = zip::ZipWriter::new(tempfile::NamedTempFile::new_in(parent)?);
        let digits = self.names.len().to_string().len().max(4);
        for (index, name) in self.names.iter().enumerate() {
            ready()?;
            let stem = Path::new(name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("page");
            let stem: String = stem
                .chars()
                .map(|c| {
                    if c.is_control()
                        || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
                    {
                        '_'
                    } else {
                        c
                    }
                })
                .collect();
            zip.start_file(
                format!(
                    "{:0digits$}_{}.jpg",
                    index + 1,
                    if stem.is_empty() { "page" } else { &stem }
                ),
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )?;
            std::io::copy(
                &mut std::fs::File::open(stage.join(format!("{index}.jpg")))
                    .context("a rendered page is missing")?,
                &mut zip,
            )?;
        }
        if let Some(comic_info) = &self.comic_info {
            zip.start_file(
                "ComicInfo.xml",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )?;
            zip.write_all(comic_info)?;
        }
        let temporary = zip.finish()?;
        temporary.as_file().sync_all()?;
        publish(temporary, destination, overwrite)
    }
}

fn read_comic_info(path: &Path) -> Result<Option<Vec<u8>>> {
    const LIMIT: u64 = 2 * 1024 * 1024;
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let is_metadata = entry
            .name()
            .ok()
            .and_then(|name| {
                Path::new(name.as_ref())
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.eq_ignore_ascii_case("ComicInfo.xml"))
            })
            .unwrap_or(false);
        if entry.is_file() && is_metadata {
            ensure!(
                entry.size() <= LIMIT,
                "ComicInfo.xml exceeds the 2 MiB metadata limit"
            );
            let mut bytes = Vec::new();
            entry.by_ref().take(LIMIT + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= LIMIT,
                "ComicInfo.xml exceeds the 2 MiB metadata limit"
            );
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_pages_in_order_and_replaces_only_after_a_complete_archive() -> Result<()> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source.cbz");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&source)?);
        let options = zip::write::SimpleFileOptions::default();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2))
            .write_to(&mut png, image::ImageFormat::Png)?;
        for name in ["page10.png", "page2.png"] {
            zip.start_file(name, options)?;
            zip.write_all(png.get_ref())?;
        }
        zip.start_file("ComicInfo.xml", options)?;
        zip.write_all(b"<ComicInfo/>")?;
        zip.finish()?;
        let chapter = Chapter::open(&source)?;
        assert_eq!(chapter.names, ["page2.png", "page10.png"]);
        let stage = tempfile::tempdir()?;
        std::fs::write(stage.path().join("0.jpg"), b"first")?;
        let output = root.path().join("translated.cbz");
        std::fs::write(&output, b"previous")?;
        let publish = |file: tempfile::NamedTempFile, path: &Path, overwrite: bool| -> Result<()> {
            if overwrite {
                file.persist(path)?;
            } else {
                file.persist_noclobber(path)?;
            }
            Ok(())
        };
        assert!(
            chapter
                .publish(stage.path(), &output, true, || Ok(()), publish)
                .is_err()
        );
        assert_eq!(std::fs::read(&output)?, b"previous");
        std::fs::write(stage.path().join("1.jpg"), b"second")?;
        chapter.publish(stage.path(), &output, true, || Ok(()), publish)?;
        let mut result = zip::ZipArchive::new(std::fs::File::open(output)?)?;
        assert_eq!(result.by_index(0)?.name()?.as_ref(), "0001_page2.jpg");
        assert_eq!(result.by_index(1)?.name()?.as_ref(), "0002_page10.jpg");
        let mut xml = String::new();
        result.by_name("ComicInfo.xml")?.read_to_string(&mut xml)?;
        assert_eq!(xml, "<ComicInfo/>");
        Ok(())
    }
}
