use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::{Context as _, Result};
use koharu_scene::{AssetRole, BlobId, EntityId, Snapshot};

pub(crate) struct ImageCache {
    decoded: Mutex<HashMap<BlobId, CachedImage>>,
    max_bytes: usize,
    max_entries: usize,
}

struct CachedImage {
    decoded: Arc<tokio::sync::OnceCell<Arc<image::DynamicImage>>>,
    last_used: Instant,
    bytes: usize,
}

impl Default for ImageCache {
    fn default() -> Self {
        Self {
            decoded: Mutex::default(),
            max_bytes: 128 * 1024 * 1024,
            max_entries: 64,
        }
    }
}

impl ImageCache {
    pub(crate) async fn get(
        &self,
        scene: &Snapshot,
        entity: EntityId,
        role: &str,
    ) -> Result<Option<Arc<image::DynamicImage>>> {
        let role = AssetRole::new(role)?;
        let Some(asset) = scene.asset(entity, &role)? else {
            return Ok(None);
        };
        let cell = {
            let mut decoded = self
                .decoded
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let cached = decoded.entry(asset.blob).or_insert_with(|| CachedImage {
                decoded: Arc::default(),
                last_used: Instant::now(),
                bytes: 0,
            });
            cached.last_used = Instant::now();
            cached.decoded.clone()
        };
        self.trim();
        let image = cell
            .get_or_try_init(|| async {
                let bytes = scene.read_blob(asset.blob).await?;
                let role = role.clone();
                tokio_rayon::spawn(move || {
                    image::load_from_memory(&bytes)
                        .map(Arc::new)
                        .with_context(|| {
                            format!(
                                "failed to decode {} image for entity {entity}",
                                role.as_str()
                            )
                        })
                })
                .await
            })
            .await
            .cloned();
        if let Ok(image) = &image {
            let mut decoded = self
                .decoded
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(cached) = decoded.get_mut(&asset.blob)
                && Arc::ptr_eq(&cached.decoded, &cell)
            {
                cached.bytes = image.as_bytes().len();
            }
        }
        self.trim();
        image.map(Some)
    }

    fn trim(&self) {
        let mut decoded = self
            .decoded
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut bytes = decoded.values().map(|entry| entry.bytes).sum::<usize>();
        // Running stages retain their own Arcs. Eviction bounds idle decoded
        // images without tying upstream progress to a cloud request's lifetime.
        while bytes > self.max_bytes || decoded.len() > self.max_entries {
            let Some(oldest) = decoded
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(blob, _)| *blob)
            else {
                break;
            };
            if let Some(entry) = decoded.remove(&oldest) {
                bytes -= entry.bytes;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, RgbImage};
    use koharu_scene::{AssetInput, AssetMetadata, At, PageDraft, Session};

    async fn pages() -> (Snapshot, [EntityId; 2]) {
        let mut session = Session::memory().await.unwrap();
        let mut edit = session.snapshot().edit();
        let mut pages = Vec::new();
        for color in [0, 255] {
            let page = edit
                .add_page(PageDraft::new("page", 16.0, 16.0), At::End)
                .unwrap();
            let mut bytes = std::io::Cursor::new(Vec::new());
            DynamicImage::ImageRgb8(RgbImage::from_pixel(16, 16, image::Rgb([color; 3])))
                .write_to(&mut bytes, ImageFormat::Png)
                .unwrap();
            edit.set_asset(
                page,
                &AssetRole::new("source").unwrap(),
                AssetInput::new(
                    Arc::<[u8]>::from(bytes.into_inner()),
                    "image/png",
                    AssetMetadata {
                        width: Some(16),
                        height: Some(16),
                        attributes: Default::default(),
                    },
                ),
            )
            .unwrap();
            pages.push(page);
        }
        session.commit(edit.finish().unwrap()).await.unwrap();
        (session.snapshot(), pages.try_into().unwrap())
    }

    #[tokio::test]
    async fn evicts_decoded_images_without_invalidating_running_readers() {
        let (scene, pages) = pages().await;
        let cache = ImageCache {
            max_bytes: 1024,
            max_entries: 2,
            ..Default::default()
        };
        let first = cache
            .get(&scene, pages[0], "source")
            .await
            .unwrap()
            .unwrap();
        let (left, right) = tokio::join!(
            cache.get(&scene, pages[0], "source"),
            cache.get(&scene, pages[0], "source"),
        );
        assert!(Arc::ptr_eq(
            &left.unwrap().unwrap(),
            &right.unwrap().unwrap()
        ));
        cache.get(&scene, pages[1], "source").await.unwrap();
        assert_eq!(cache.decoded.lock().unwrap().len(), 1);
        assert_eq!(first.as_rgb8().unwrap().get_pixel(0, 0).0, [0; 3]);
        let reloaded = cache
            .get(&scene, pages[0], "source")
            .await
            .unwrap()
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &reloaded));
        assert_eq!(first.as_bytes(), reloaded.as_bytes());
    }

    #[tokio::test]
    async fn oversized_image_is_available_to_its_reader_but_not_retained() {
        let (scene, pages) = pages().await;
        let cache = ImageCache {
            max_bytes: 1,
            ..Default::default()
        };
        let image = cache
            .get(&scene, pages[0], "source")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(image.width(), 16);
        assert!(cache.decoded.lock().unwrap().is_empty());
    }
}
