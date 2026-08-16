use anyhow::{Context as _, Result};
use futures::{StreamExt as _, TryStreamExt as _, stream};
use image::{ExtendedColorType, ImageFormat, RgbImage, RgbaImage, codecs::jpeg::JpegEncoder};
use koharu_psd::{PsdExportOptions, export_page as export_psd_page};
use koharu_rasterizer::{Raster, RasterOptions, Rasterizer};
use koharu_renderer::{Frame, Renderer};
use koharu_scene::{AssetRole, EntityId, Snapshot};
use serde::Deserialize;
use specta::Type;
use std::{
    io::{Seek, Write},
    path::PathBuf,
    sync::Arc,
};
use tauri::{Cef, State, WebviewWindow, ipc::IpcResponse};
use zip::{ZipWriter, write::SimpleFileOptions};

use super::{Error, project::CurrentProject};
use koharu_desktop::Desktop;

const THUMBNAIL_EDGE: u32 = 128;
pub(crate) const CBZ_JPEG_QUALITY: u8 = 95;

#[derive(Type)]
#[specta(transparent)]
pub(crate) struct ThumbnailBytes(#[specta(type = Vec<u8>)] Vec<u8>);

impl IpcResponse for ThumbnailBytes {
    fn body(self) -> tauri::Result<tauri::ipc::InvokeResponseBody> {
        Ok(self.0.into())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PageExportFormat {
    Png,
    Psd,
}

#[derive(Clone, Copy, Debug, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProjectExportFormat {
    Png,
    Psd,
    Cbz,
}

#[derive(Clone, Copy, Debug, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExportRequest {
    CurrentPage {
        page: EntityId,
        format: PageExportFormat,
    },
    EntireProject {
        format: ProjectExportFormat,
    },
}

struct ExportPage {
    id: EntityId,
    stem: String,
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn export_pages(
    window: WebviewWindow<Cef>,
    request: ExportRequest,
    project: State<'_, CurrentProject>,
    desktop: State<'_, Desktop>,
) -> std::result::Result<(), Error> {
    let (snapshot, project_name) = {
        let project = project.project.lock().await;
        let project = project.as_ref().context("no project is open")?;
        (project.snapshot(), project.info().name)
    };
    match request {
        ExportRequest::CurrentPage { page, format } => {
            let page = export_page_job(&snapshot, page, None)?;
            let (label, extension) = page_format(format);
            let Some(path) = rfd::AsyncFileDialog::new()
                .set_parent(&window)
                .add_filter(label, &[extension])
                .set_file_name(format!("{}.{}", page.stem, extension))
                .save_file()
                .await
                .map(|file| file.path().to_owned())
            else {
                return Ok(());
            };
            let renderer = desktop.renderer();
            let rasterizer = desktop.rasterizer().await?;
            export_page_file(
                &snapshot,
                &renderer,
                Arc::clone(&rasterizer),
                page.id,
                format,
                path,
            )
            .await?;
        }
        ExportRequest::EntireProject { format } => {
            let pages = snapshot
                .pages()
                .enumerate()
                .map(|(index, page)| export_page_job(&snapshot, page.id(), Some(index)))
                .collect::<Result<Vec<_>>>()?;
            if pages.is_empty() {
                return Err(anyhow::anyhow!("there are no pages to export").into());
            }
            match format {
                ProjectExportFormat::Png | ProjectExportFormat::Psd => {
                    let Some(parent) = rfd::AsyncFileDialog::new()
                        .set_parent(&window)
                        .pick_folder()
                        .await
                        .map(|directory| directory.path().to_owned())
                    else {
                        return Ok(());
                    };
                    let directory = parent.join(&project_name);
                    tokio::fs::create_dir_all(&directory).await?;
                    let renderer = desktop.renderer();
                    let rasterizer = desktop.rasterizer().await?;
                    let format = match format {
                        ProjectExportFormat::Png => PageExportFormat::Png,
                        ProjectExportFormat::Psd => PageExportFormat::Psd,
                        ProjectExportFormat::Cbz => unreachable!(),
                    };
                    export_project_files(snapshot, renderer, rasterizer, pages, format, directory)
                        .await?;
                }
                ProjectExportFormat::Cbz => {
                    let Some(path) = rfd::AsyncFileDialog::new()
                        .set_parent(&window)
                        .add_filter("Comic Book Archive", &["cbz"])
                        .set_file_name(format!("{project_name}.cbz"))
                        .save_file()
                        .await
                        .map(|file| file.path().to_owned())
                    else {
                        return Ok(());
                    };
                    let renderer = desktop.renderer();
                    let rasterizer = desktop.rasterizer().await?;
                    export_project_cbz(
                        snapshot,
                        renderer,
                        rasterizer,
                        pages,
                        path,
                        CBZ_JPEG_QUALITY,
                        None,
                    )
                    .await?;
                }
            }
        }
    }
    Ok(())
}

async fn export_project_files(
    snapshot: Snapshot,
    renderer: Renderer,
    rasterizer: Arc<Rasterizer>,
    pages: Vec<ExportPage>,
    format: PageExportFormat,
    directory: PathBuf,
) -> Result<()> {
    stream::iter(pages)
        .map(|page| {
            let renderer = renderer.clone();
            let rasterizer = Arc::clone(&rasterizer);
            let snapshot = snapshot.clone();
            let directory = directory.clone();
            async move {
                let (_, extension) = page_format(format);
                export_page_file(
                    &snapshot,
                    &renderer,
                    rasterizer,
                    page.id,
                    format,
                    directory.join(format!("{}.{}", page.stem, extension)),
                )
                .await
            }
        })
        .buffer_unordered(4)
        .try_collect::<Vec<_>>()
        .await?;
    Ok(())
}

async fn export_page_file(
    snapshot: &Snapshot,
    renderer: &Renderer,
    rasterizer: Arc<Rasterizer>,
    page: EntityId,
    format: PageExportFormat,
    path: PathBuf,
) -> Result<()> {
    let frame = renderer.render(snapshot, page).await?;
    match format {
        PageExportFormat::Png => {
            let image = rasterize(rasterizer, &frame, RasterOptions::default())
                .await?
                .image;
            tokio::task::spawn_blocking(move || image.save_with_format(path, ImageFormat::Png))
                .await
                .context("PNG export worker stopped unexpectedly")??;
        }
        PageExportFormat::Psd => {
            let bytes =
                export_psd_page(rasterizer, snapshot, &frame, &PsdExportOptions::default()).await?;
            tokio::fs::write(path, bytes).await?;
        }
    }
    Ok(())
}

async fn export_project_cbz(
    snapshot: Snapshot,
    renderer: Renderer,
    rasterizer: Arc<Rasterizer>,
    pages: Vec<ExportPage>,
    path: PathBuf,
    jpeg_quality: u8,
    comic_info: Option<Vec<u8>>,
) -> Result<()> {
    anyhow::ensure!(
        (1..=100).contains(&jpeg_quality),
        "JPEG quality must be between 1 and 100"
    );
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    tokio::fs::create_dir_all(parent).await?;
    let temporary = tempfile::NamedTempFile::new_in(parent).with_context(|| {
        format!(
            "failed to create a temporary archive in {}",
            parent.display()
        )
    })?;
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<(String, Vec<u8>)>(2);
    let archive_worker = tokio::task::spawn_blocking(move || -> Result<tempfile::NamedTempFile> {
        let mut archive = ZipWriter::new(temporary);
        while let Some((name, bytes)) = receiver.blocking_recv() {
            append_cbz_entry(&mut archive, &name, &bytes)?;
        }
        if let Some(comic_info) = comic_info {
            append_cbz_entry(&mut archive, "ComicInfo.xml", &comic_info)?;
        }
        let temporary = archive.finish()?;
        temporary.as_file().sync_all()?;
        Ok(temporary)
    });

    let encoded_pages = stream::iter(pages)
        .map(|page| {
            let renderer = renderer.clone();
            let rasterizer = Arc::clone(&rasterizer);
            let snapshot = snapshot.clone();
            async move {
                let frame = renderer.render(&snapshot, page.id).await?;
                let image = rasterize(rasterizer, &frame, RasterOptions::default())
                    .await?
                    .image;
                let bytes = tokio::task::spawn_blocking(move || encode_jpeg(image, jpeg_quality))
                    .await
                    .context("JPEG encode worker stopped unexpectedly")??;
                Ok::<_, anyhow::Error>((format!("{}.jpg", page.stem), bytes))
            }
        })
        .buffered(4);
    futures::pin_mut!(encoded_pages);

    let export_result = async {
        while let Some(entry) = encoded_pages.try_next().await? {
            sender
                .send(entry)
                .await
                .map_err(|_| anyhow::anyhow!("CBZ archive writer stopped unexpectedly"))?;
        }
        Ok::<_, anyhow::Error>(())
    }
    .await
    .context("failed to prepare CBZ pages");
    drop(sender);

    let temporary = archive_worker
        .await
        .context("CBZ archive writer stopped unexpectedly")?;
    let temporary = temporary?;
    export_result?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to publish {}", path.display()))?;
    Ok(())
}

pub(crate) async fn export_snapshot_cbz(
    snapshot: Snapshot,
    renderer: Renderer,
    rasterizer: Arc<Rasterizer>,
    path: PathBuf,
    jpeg_quality: u8,
    comic_info: Option<Vec<u8>>,
) -> Result<()> {
    let pages = snapshot
        .pages()
        .enumerate()
        .map(|(index, page)| export_page_job(&snapshot, page.id(), Some(index)))
        .collect::<Result<Vec<_>>>()?;
    anyhow::ensure!(!pages.is_empty(), "there are no pages to export");
    export_project_cbz(
        snapshot,
        renderer,
        rasterizer,
        pages,
        path,
        jpeg_quality,
        comic_info,
    )
    .await
}

fn export_page_job(snapshot: &Snapshot, id: EntityId, index: Option<usize>) -> Result<ExportPage> {
    let page = snapshot.page(id)?.page()?;
    let stem = page_stem(&page.label);
    Ok(ExportPage {
        id,
        stem: index.map_or(stem.clone(), |index| format!("{:04}_{stem}", index + 1)),
    })
}

fn page_stem(label: &str) -> String {
    let label = label
        .trim()
        .trim_end_matches(|character: char| character == '.' || character.is_whitespace());
    let label = label.rsplit_once('.').map_or(label, |(stem, _)| stem);
    let stem = label
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    if stem.is_empty() {
        "page".to_owned()
    } else {
        stem
    }
}

fn page_format(format: PageExportFormat) -> (&'static str, &'static str) {
    match format {
        PageExportFormat::Png => ("PNG Image", "png"),
        PageExportFormat::Psd => ("Photoshop Document", "psd"),
    }
}

fn encode_jpeg(image: RgbaImage, quality: u8) -> Result<Vec<u8>> {
    anyhow::ensure!(
        (1..=100).contains(&quality),
        "JPEG quality must be between 1 and 100"
    );
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
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    for pixel in image.pixels() {
        let alpha = u16::from(pixel[3]);
        let inverse = 255 - alpha;
        for channel in &pixel.0[..3] {
            pixels.push(((u16::from(*channel) * alpha + 255 * inverse + 127) / 255) as u8);
        }
    }
    RgbImage::from_raw(width, height, pixels).expect("RGB buffer dimensions are exact")
}

fn append_cbz_entry<W: Write + Seek>(
    archive: &mut ZipWriter<W>,
    name: &str,
    bytes: &[u8],
) -> Result<()> {
    archive.start_file(
        name,
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
    )?;
    archive.write_all(bytes)?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn get_thumbnail(
    page: EntityId,
    project: State<'_, CurrentProject>,
) -> std::result::Result<ThumbnailBytes, Error> {
    let snapshot = project
        .project
        .lock()
        .await
        .as_ref()
        .context("no project is open")?
        .snapshot();
    snapshot.page(page)?;
    let blob = snapshot
        .asset(page, &AssetRole::new("source")?)?
        .with_context(|| format!("page {page} has no source image"))?
        .blob;
    let bytes = snapshot.read_blob(blob).await?;
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let image = image::load_from_memory(&bytes).context("failed to decode source image")?;
        if image.width() == 0 || image.height() == 0 {
            return Err(anyhow::anyhow!("source image is empty"));
        }
        let image = image.thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE).to_rgba8();
        let encoder = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height());
        Ok(encoder.encode(80.0).to_vec())
    })
    .await
    .context("thumbnail worker stopped unexpectedly")??;
    Ok(ThumbnailBytes(bytes))
}

pub(crate) async fn rendered_preview(
    renderer: &Renderer,
    rasterizer: Arc<Rasterizer>,
    snapshot: &Snapshot,
    page: EntityId,
) -> Result<Vec<u8>> {
    snapshot.page(page)?;
    let frame = renderer.render(snapshot, page).await?;
    let image = rasterize(rasterizer, &frame, RasterOptions::default())
        .await?
        .image;
    tokio::task::spawn_blocking(move || {
        let image = image::DynamicImage::ImageRgba8(image)
            .resize(1024, 1024, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let encoder = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height());
        Ok::<_, anyhow::Error>(encoder.encode(85.0).to_vec())
    })
    .await
    .context("preview encode worker stopped unexpectedly")?
}

async fn rasterize(
    rasterizer: Arc<Rasterizer>,
    frame: &Frame,
    options: RasterOptions,
) -> Result<Raster> {
    let frame = frame.raster_frame()?;
    tokio::task::spawn_blocking(move || rasterizer.rasterize(&frame, options))
        .await
        .context("rasterizer worker stopped unexpectedly")?
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read as _};

    use image::RgbaImage;

    use super::*;

    #[test]
    fn creates_safe_page_file_stems() {
        assert_eq!(page_stem("  Chapter 1.png  "), "Chapter 1");
        assert_eq!(page_stem("page:01?.webp"), "page_01_");
        assert_eq!(page_stem("..."), "page");
    }

    #[test]
    fn writes_compact_jpeg_images_into_cbz_entries() {
        let mut source = RgbaImage::new(2, 3);
        source.get_pixel_mut(0, 0).0 = [255, 0, 0, 128];
        let jpeg = encode_jpeg(source, CBZ_JPEG_QUALITY).unwrap();
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        append_cbz_entry(&mut writer, "0001_Page 1.jpg", &jpeg).unwrap();

        let cursor = writer.finish().unwrap();
        let mut archive = zip::ZipArchive::new(cursor).unwrap();
        assert_eq!(archive.len(), 1);
        let mut entry = archive.by_index(0).unwrap();
        assert_eq!(entry.name(), "0001_Page 1.jpg");
        assert_eq!(entry.compression(), zip::CompressionMethod::Stored);
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        let image = image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg).unwrap();
        assert_eq!((image.width(), image.height()), (2, 3));
    }

    #[test]
    fn composites_transparency_onto_white_before_jpeg_encoding() {
        let mut source = RgbaImage::new(1, 1);
        source.get_pixel_mut(0, 0).0 = [0, 0, 0, 0];

        assert_eq!(composite_on_white(source).get_pixel(0, 0).0, [255; 3]);
    }
}
