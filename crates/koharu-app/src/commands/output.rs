use anyhow::{Context as _, Result};
use futures::{StreamExt as _, TryStreamExt as _, stream};
use image::{
    ExtendedColorType, ImageEncoder as _,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use koharu_psd::{PsdExportOptions, export_page};
use koharu_rasterizer::{Raster, RasterOptions, Rasterizer};
use koharu_renderer::{Frame, Renderer};
use koharu_scene::{AssetRole, EntityId, Snapshot};
use serde::Deserialize;
use specta::Type;
use std::sync::Arc;
use tauri::{State, WebviewWindow, ipc::IpcResponse};
use tauri_runtime_cef::CefRuntime;

use super::{Error, project::CurrentProject};
use koharu_desktop::Desktop;

pub(super) mod cbz;

const THUMBNAIL_EDGE: u32 = 128;

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
pub enum ProjectExportFormat {
    Png,
    Psd,
    Cbz,
}

#[derive(Clone, Copy, Debug, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PageExportFormat {
    Png,
    Psd,
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

#[tracing::instrument(
    target = "koharu_metrics",
    name = "export",
    skip_all,
    fields(origin = "user", request = ?request),
)]
#[tauri::command]
#[specta::specta]
pub(crate) async fn export(
    window: WebviewWindow<CefRuntime>,
    request: ExportRequest,
    project: State<'_, CurrentProject>,
    desktop: State<'_, Desktop>,
) -> std::result::Result<(), Error> {
    let (name, snapshot) = {
        let project = project.project.lock().await;
        let project = project.as_ref().context("no project is open")?;
        (project.name.clone(), project.snapshot())
    };
    let (format, current_page) = match request {
        ExportRequest::CurrentPage { page, format } => (
            match format {
                PageExportFormat::Png => ProjectExportFormat::Png,
                PageExportFormat::Psd => ProjectExportFormat::Psd,
            },
            Some(page),
        ),
        ExportRequest::EntireProject { format } => (format, None),
    };
    let pages = if let Some(page) = current_page {
        snapshot.page(page)?.page()?;
        vec![page]
    } else {
        snapshot.pages().map(|page| page.id()).collect::<Vec<_>>()
    };
    if pages.is_empty() {
        return Err(anyhow::anyhow!("there are no pages to export").into());
    }
    let dialog = rfd::AsyncFileDialog::new().set_parent(&window);
    let destination = match (current_page, format) {
        (Some(page), format) => {
            let extension = match format {
                ProjectExportFormat::Png => "png",
                ProjectExportFormat::Psd => "psd",
                ProjectExportFormat::Cbz => unreachable!(),
            };
            let label = page_stem(&snapshot.page(page)?.page()?.label);
            dialog
                .add_filter(extension.to_uppercase(), &[extension])
                .set_file_name(format!("{label}.{extension}"))
                .save_file()
                .await
        }
        (None, ProjectExportFormat::Png | ProjectExportFormat::Psd) => dialog.pick_folder().await,
        (None, ProjectExportFormat::Cbz) => {
            dialog
                .add_filter("Comic Book Archive", &["cbz"])
                .set_file_name(format!("{name}.cbz"))
                .save_file()
                .await
        }
    };
    let Some(destination) = destination.map(|destination| destination.path().to_owned()) else {
        return Ok(());
    };
    let destination = if current_page.is_none() && !matches!(format, ProjectExportFormat::Cbz) {
        let directory = destination.join(name);
        tokio::fs::create_dir_all(&directory).await?;
        directory
    } else {
        destination
    };
    let renderer = desktop.renderer();
    let rasterizer = desktop.rasterizer().await?;
    if matches!(format, ProjectExportFormat::Cbz) {
        let ready = stream::iter(pages.into_iter().map(|page| Ok((page, snapshot.clone()))));
        let prepared = cbz::prepare(
            snapshot.clone(),
            ready,
            renderer,
            rasterizer,
            cbz::DEFAULT_QUALITY,
            koharu_pipeline::RunControl::default(),
            None,
        )
        .await?;
        prepared.write(destination, None, true).await?;
        return Ok(());
    }
    let width = pages.len().to_string().len().max(4);
    stream::iter(pages.into_iter().enumerate()).map(|(index, page)| {
        let snapshot = snapshot.clone();
        let renderer = renderer.clone();
        let rasterizer = Arc::clone(&rasterizer);
        let destination = destination.clone();
        async move {
            let stem = page_stem(&snapshot.page(page)?.page()?.label);
            let frame = renderer.render(&snapshot, page).await?;
            let (extension, bytes) = match format {
                ProjectExportFormat::Png => {
                    let image = rasterize(rasterizer, &frame, RasterOptions::default()).await?.image;
                    let bytes = tokio_rayon::spawn(move || -> Result<Vec<u8>> {
                        let mut bytes = Vec::new();
                        PngEncoder::new_with_quality(&mut bytes, CompressionType::Best, FilterType::Adaptive)
                            .write_image(image.as_raw(), image.width(), image.height(), ExtendedColorType::Rgba8)?;
                        Ok(bytes)
                    }).await?;
                    ("png", bytes)
                }
                ProjectExportFormat::Psd => ("psd", export_page(rasterizer, &snapshot, &frame, &PsdExportOptions::default()).await?),
                ProjectExportFormat::Cbz => unreachable!(),
            };
            let path = if current_page.is_some() { destination } else { destination.join(format!("{:0width$}_{stem}.{extension}", index + 1)) };
            tokio::fs::write(path, bytes).await?;
            tracing::info!(target: "koharu_metrics", metric = "page_exported", format = ?format);
            Ok::<_, anyhow::Error>(())
        }
    }).buffer_unordered(4).try_collect::<Vec<_>>().await?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_filenames_remove_extensions_and_path_characters() {
        assert_eq!(page_stem("page: 2/3.png"), "page_ 2_3");
        assert_eq!(page_stem("  ...  "), "page");
        assert_eq!(page_stem("front.page.png"), "front.page");
    }
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
    let bytes = tokio_rayon::spawn(move || -> Result<Vec<u8>> {
        let image = image::load_from_memory(&bytes).context("failed to decode source image")?;
        if image.width() == 0 || image.height() == 0 {
            return Err(anyhow::anyhow!("source image is empty"));
        }
        let image = image.thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE).to_rgba8();
        let encoder = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height());
        Ok(encoder.encode(80.0).to_vec())
    })
    .await?;
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
    tokio_rayon::spawn(move || {
        let image = image::DynamicImage::ImageRgba8(image)
            .resize(1024, 1024, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let encoder = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height());
        Ok::<_, anyhow::Error>(encoder.encode(85.0).to_vec())
    })
    .await
}

async fn rasterize(
    rasterizer: Arc<Rasterizer>,
    frame: &Frame,
    options: RasterOptions,
) -> Result<Raster> {
    let frame = frame.raster_frame()?;
    tokio_rayon::spawn(move || rasterizer.rasterize(&frame, options))
        .await
        .map_err(Into::into)
}
