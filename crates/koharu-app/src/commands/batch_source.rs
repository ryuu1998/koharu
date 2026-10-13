use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rayon::prelude::*;
use serde::Serialize;
use specta::Type;
use tauri::WebviewWindow;
use tauri_runtime_cef::CefRuntime;

use super::{Error, import::Format};

const COVER_LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Type)]
pub struct BatchSource {
    pub path: PathBuf,
    pub default_output: PathBuf,
    pub chapters: Vec<BatchChapter>,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct BatchChapter {
    pub path: PathBuf,
    pub name: String,
    #[specta(type = f64)]
    pub size: u64,
    #[specta(type = f64)]
    pub pages: usize,
    pub thumbnail: Option<String>,
    pub error: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn browse_batch_source(
    window: WebviewWindow<CefRuntime>,
) -> std::result::Result<Option<BatchSource>, Error> {
    let Some(path) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .pick_folder()
        .await
        .map(|folder| folder.path().to_owned())
    else {
        return Ok(None);
    };
    Ok(Some(tokio_rayon::spawn(move || scan(path)).await?))
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn browse_batch_output(
    window: WebviewWindow<CefRuntime>,
) -> std::result::Result<Option<PathBuf>, Error> {
    Ok(rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .pick_folder()
        .await
        .map(|folder| folder.path().to_owned()))
}

pub(super) fn discover(input: &Path) -> Result<Vec<PathBuf>> {
    let mut chapters = if input.is_file() {
        ensure!(is_cbz(input), "input must be a CBZ archive");
        vec![input.to_owned()]
    } else {
        ensure!(
            input.is_dir(),
            "input folder does not exist: {}",
            input.display()
        );
        fs::read_dir(input)?
            .filter_map(|entry| match entry {
                Ok(entry)
                    if entry.file_type().is_ok_and(|kind| kind.is_file())
                        && is_cbz(&entry.path()) =>
                {
                    Some(Ok(entry.path()))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<std::io::Result<Vec<_>>>()?
    };
    alphanumeric_sort::sort_slice_by_path_key(&mut chapters, PathBuf::as_path);
    Ok(chapters)
}

pub(super) fn is_cbz(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cbz"))
}

fn scan(path: PathBuf) -> Result<BatchSource> {
    let chapters = discover(&path)?
        .par_iter()
        .map(|path| {
            let mut chapter = BatchChapter {
                path: path.clone(),
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                size: fs::metadata(path).map_or(0, |metadata| metadata.len()),
                pages: 0,
                thumbnail: None,
                error: None,
            };
            match inspect(path) {
                Ok((pages, thumbnail)) => {
                    chapter.pages = pages;
                    chapter.thumbnail = Some(thumbnail);
                }
                Err(error) => chapter.error = Some(format!("{error:#}")),
            }
            chapter
        })
        .collect();
    Ok(BatchSource {
        default_output: path.join("Translated"),
        path,
        chapters,
    })
}

fn inspect(path: &Path) -> Result<(usize, String)> {
    let mut archive = zip::ZipArchive::new(fs::File::open(path)?)?;
    let mut images = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let path = Path::new(entry.name());
        let raster = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| matches!(extension.parse(), Ok(Format::Raster)));
        let metadata = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|name| name.starts_with("._"))
            || path
                .components()
                .any(|part| part.as_os_str().eq_ignore_ascii_case("__MACOSX"));
        if entry.is_file() && raster && !metadata {
            images.push((index, entry.name().to_owned()));
        }
    }
    alphanumeric_sort::sort_slice_by_str_key(&mut images, |(_, name)| name);
    let (index, _) = images.first().context("archive contains no page images")?;
    let mut cover = archive.by_index(*index)?;
    ensure!(
        cover.size() <= COVER_LIMIT,
        "cover exceeds the 512 MiB import limit"
    );
    let mut bytes = Vec::new();
    cover
        .by_ref()
        .take(COVER_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= COVER_LIMIT,
        "cover exceeds the 512 MiB import limit"
    );
    let image = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .decode()?
        .thumbnail(96, 128)
        .to_rgba8();
    let thumbnail =
        webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height()).encode(75.0);
    Ok((
        images.len(),
        format!("data:image/webp;base64,{}", STANDARD.encode(&*thumbnail)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn scans_natural_order_with_a_cover_and_reports_corrupt_archives() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chapter2.cbz");
        let mut archive = zip::ZipWriter::new(fs::File::create(path).unwrap());
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 3))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("page10.png", options).unwrap();
        archive.write_all(bytes.get_ref()).unwrap();
        archive.start_file("page2.png", options).unwrap();
        archive.write_all(bytes.get_ref()).unwrap();
        archive.start_file("ComicInfo.xml", options).unwrap();
        archive.write_all(b"<ComicInfo />").unwrap();
        archive.finish().unwrap();
        fs::write(directory.path().join("chapter10.CBZ"), b"broken").unwrap();
        fs::write(directory.path().join("notes.txt"), b"ignore").unwrap();
        let source = scan(directory.path().to_owned()).unwrap();
        assert_eq!(source.chapters.len(), 2);
        assert_eq!(source.chapters[0].name, "chapter2.cbz");
        assert_eq!(source.chapters[0].pages, 2);
        assert!(
            source.chapters[0]
                .thumbnail
                .as_ref()
                .unwrap()
                .starts_with("data:image/webp;base64,")
        );
        assert!(source.chapters[1].error.is_some());
    }
}
