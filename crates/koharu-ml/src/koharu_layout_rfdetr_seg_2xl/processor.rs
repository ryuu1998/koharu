//! RF-DETR image preprocessing and instance segmentation postprocessing.
//!
//! https://github.com/roboflow/rf-detr/blob/4ab7c18729de9d02ffd0495795d0831b5630f01b/src/rfdetr/detr.py
//! https://github.com/roboflow/rf-detr/blob/4ab7c18729de9d02ffd0495795d0831b5630f01b/src/rfdetr/models/postprocess.py

use anyhow::{Result, ensure};
use image::DynamicImage;
use koharu_torch::{Device, IndexOp, Kind, Tensor};
use serde::Serialize;

use super::{
    config::{KoharuLayoutRFDetrSeg2XLConfig, KoharuLayoutThresholds},
    model::Output,
};

const MASK_CHUNK_BUDGET: usize = 32 * 1024 * 1024;
const MAX_MASK_CHUNK: usize = 8;

#[derive(Debug, Clone)]
pub struct KoharuLayoutRFDetrImageProcessor {
    resolution: i64,
    num_select: i64,
    class_names: Vec<String>,
    recommended_thresholds: KoharuLayoutThresholds,
}

impl KoharuLayoutRFDetrImageProcessor {
    pub fn new(config: &KoharuLayoutRFDetrSeg2XLConfig) -> Result<Self> {
        ensure!(config.resolution > 0, "RF-DETR resolution must be positive");
        ensure!(
            config.resolution % 24 == 0,
            "RF-DETR resolution must be divisible by patch_size * num_windows"
        );
        ensure!(config.num_select > 0, "RF-DETR num_select must be positive");
        Ok(Self {
            resolution: config.resolution,
            num_select: config.num_select,
            class_names: config.class_names(),
            recommended_thresholds: config.recommended_thresholds,
        })
    }

    pub(super) fn recommended_thresholds(&self) -> KoharuLayoutThresholds {
        self.recommended_thresholds
    }

    pub(super) fn preprocess(&self, image: &DynamicImage, device: Device) -> Result<Tensor> {
        ensure!(
            image.width() > 0 && image.height() > 0,
            "cannot segment an empty image"
        );
        let image = image.to_rgb8();
        let mut pixel_values = Tensor::from_slice(image.as_raw())
            .view([1, i64::from(image.height()), i64::from(image.width()), 3])
            .permute([0, 3, 1, 2])
            .to_device(device)
            .to_kind(Kind::Float)
            / 255.0;
        if image.width() != self.resolution as u32 || image.height() != self.resolution as u32 {
            // Latest upstream disables antialiasing to match the bilinear
            // Albumentations resize used during training.
            pixel_values = pixel_values.upsample_bilinear2d(
                [self.resolution, self.resolution],
                false,
                None,
                None,
            );
        }
        let mean = Tensor::from_slice(&[0.485f32, 0.456, 0.406])
            .view([1, 3, 1, 1])
            .to_device(device);
        let std = Tensor::from_slice(&[0.229f32, 0.224, 0.225])
            .view([1, 3, 1, 1])
            .to_device(device);
        Ok((pixel_values - mean) / std)
    }

    pub(super) fn postprocess(
        &self,
        output: &Output,
        image_width: u32,
        image_height: u32,
        thresholds: KoharuLayoutThresholds,
    ) -> Result<KoharuLayoutDetections> {
        validate_thresholds(thresholds)?;
        let logits_size = output.pred_logits.size();
        ensure!(
            logits_size == [1, 300, 5],
            "unexpected RF-DETR logits shape {logits_size:?}"
        );
        ensure!(
            output.pred_boxes.size() == [1, 300, 4],
            "unexpected RF-DETR box shape {:?}",
            output.pred_boxes.size()
        );
        ensure!(
            output.pred_masks.size() == [1, 300, 288, 288],
            "unexpected RF-DETR mask shape {:?}",
            output.pred_masks.size()
        );

        // PostProcess ranks every query/class pair, including the checkpoint's
        // fifth background logit slot, before applying the caller threshold.
        let (scores, indexes) =
            output
                .pred_logits
                .sigmoid()
                .view([1, -1])
                .topk(self.num_select, 1, true, true);
        let scores = scores.i(0);
        let indexes = indexes.i(0);
        let query_indexes = indexes.floor_divide_scalar(5);
        let labels = indexes.remainder(5);
        let thresholds = Tensor::from_slice(&thresholds.with_background())
            .to_device(scores.device())
            .index_select(0, &labels);
        let keep = scores.gt_tensor(&thresholds);
        let selected = keep.nonzero().view([-1]);

        if selected.size()[0] == 0 {
            return Ok(KoharuLayoutDetections {
                image_width,
                image_height,
                detections: Vec::new(),
            });
        }

        let scores = scores.index_select(0, &selected);
        let labels = labels.index_select(0, &selected);
        let query_indexes = query_indexes.index_select(0, &selected);
        let boxes = output.pred_boxes.i(0).index_select(0, &query_indexes);
        let center = boxes.i((.., 0..2));
        let half_size = boxes.i((.., 2..4)) / 2.0;
        let boxes = Tensor::cat(&[&center - &half_size, &center + &half_size], 1)
            * Tensor::from_slice(&[
                image_width as f32,
                image_height as f32,
                image_width as f32,
                image_height as f32,
            ])
            .to_device(output.pred_boxes.device());

        // Gathering before interpolation is output-equivalent to upstream and
        // avoids allocating resized masks for candidates rejected by threshold.
        let masks = output.pred_masks.i(0).index_select(0, &query_indexes);

        let floats = Tensor::cat(&[scores.unsqueeze(1), boxes], 1);
        let floats = tensor_to_vec_f32(&floats)?;
        let labels = tensor_to_vec_i64(&labels)?;

        let resolved = resolve_masks(&masks, image_width, image_height)?;
        let mut detections = Vec::with_capacity(labels.len());
        for (index, (label, (area, mask))) in labels.into_iter().zip(resolved).enumerate() {
            let label_id = label as usize;
            detections.push(KoharuLayoutDetection {
                label_id,
                label: self
                    .class_names
                    .get(label_id)
                    .cloned()
                    .unwrap_or_else(|| "__background__".to_owned()),
                score: floats[index * 5],
                bbox: floats[index * 5 + 1..index * 5 + 5].try_into().unwrap(),
                area,
                mask,
            });
        }

        Ok(KoharuLayoutDetections {
            image_width,
            image_height,
            detections,
        })
    }
}

fn mask_chunk_size(kind: Kind, image_width: u32, image_height: u32) -> usize {
    let width = image_width as usize;
    let height = image_height as usize;
    // Include overlapping interpolation/conversion or crop/packing buffers and
    // occupancy reductions. This bounds our work tensors, not backend workspace;
    // a single unusually large page remains the minimum unit of interpolation.
    let pixels = width.saturating_mul(height);
    let bytes = pixels
        .saturating_mul((kind.elt_size_in_bytes() + 3).max(4))
        .saturating_add(width.saturating_add(height).saturating_mul(8 * 6))
        .max(1);
    (MASK_CHUNK_BUDGET / bytes).clamp(1, MAX_MASK_CHUNK)
}

fn resolve_masks(
    masks: &Tensor,
    image_width: u32,
    image_height: u32,
) -> Result<Vec<(u32, KoharuLayoutMask)>> {
    let count = masks.size()[0] as usize;
    let chunk_size = mask_chunk_size(masks.kind(), image_width, image_height);
    let mut resolved = Vec::with_capacity(count);
    for start in (0..count).step_by(chunk_size) {
        let end = (start + chunk_size).min(count);
        // Preserve upstream's source-resolution bilinear projection and >0
        // threshold. Bounded chunks amortize CUDA launches and host reads without
        // retaining a page-sized mask for every selected detection.
        let mask = masks
            .i(start as i64..end as i64)
            .unsqueeze(1)
            .upsample_bilinear2d(
                [i64::from(image_height), i64::from(image_width)],
                false,
                None,
                None,
            )
            .gt(0.0)
            .to_kind(Kind::Uint8);
        let row_counts = mask.count_nonzero_dim_intlist(&[1i64, 3][..]);
        let areas = row_counts.sum_dim_intlist(&[1i64][..], false, Kind::Int64);
        let rows = row_counts.gt(0).to_kind(Kind::Int64);
        let columns = mask.any_dims(&[1i64, 2][..], false).to_kind(Kind::Int64);
        let occupied = Tensor::stack(
            &[
                areas,
                columns.argmax(1, false),
                rows.argmax(1, false),
                i64::from(image_width) - columns.flip([1]).argmax(1, false),
                i64::from(image_height) - rows.flip([1]).argmax(1, false),
            ],
            1,
        );
        let occupied = tensor_to_vec_i64(&occupied)?;
        let mut crops = Vec::with_capacity(end - start);
        let mut extents = Vec::with_capacity(end - start);
        for (index, occupied) in occupied.as_chunks::<5>().0.iter().enumerate() {
            let area = occupied[0].clamp(0, i64::from(u32::MAX)) as u32;
            let (x, y, right, bottom) = if area == 0 {
                (0, 0, 0, 0)
            } else {
                (
                    occupied[1] as u32,
                    occupied[2] as u32,
                    occupied[3] as u32,
                    occupied[4] as u32,
                )
            };
            let width = right.saturating_sub(x);
            let height = bottom.saturating_sub(y);
            if width > 0 && height > 0 {
                crops.push(
                    (mask.i((
                        index as i64,
                        0,
                        i64::from(y)..i64::from(bottom),
                        i64::from(x)..i64::from(right),
                    )) * 255)
                        .flatten(0, -1),
                );
            }
            extents.push((area, x, y, width, height));
        }
        // One packed pixel read follows the shared extent read for this chunk;
        // individual crops never trigger a device-to-host transfer.
        let pixels = if crops.is_empty() {
            Vec::new()
        } else {
            tensor_to_vec_u8(&Tensor::cat(&crops, 0))?
        };
        let mut offset = 0;
        for (area, x, y, width, height) in extents {
            let length = width as usize * height as usize;
            resolved.push((
                area,
                KoharuLayoutMask {
                    x,
                    y,
                    width,
                    height,
                    pixels: pixels[offset..offset + length].to_vec(),
                },
            ));
            offset += length;
        }
    }
    Ok(resolved)
}

fn validate_thresholds(thresholds: KoharuLayoutThresholds) -> Result<()> {
    for (class, threshold) in [
        ("text", thresholds.text),
        ("onomatopoeia", thresholds.onomatopoeia),
        ("bubble", thresholds.bubble),
        ("panel", thresholds.panel),
    ] {
        ensure!(
            (0.0..=1.0).contains(&threshold),
            "{class} confidence threshold must be between 0 and 1"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct KoharuLayoutDetections {
    pub image_width: u32,
    pub image_height: u32,
    pub detections: Vec<KoharuLayoutDetection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KoharuLayoutDetection {
    pub label_id: usize,
    pub label: String,
    pub score: f32,
    pub bbox: [f32; 4],
    pub area: u32,
    #[serde(skip_serializing)]
    pub mask: KoharuLayoutMask,
}

#[derive(Debug, Clone)]
pub struct KoharuLayoutMask {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl KoharuLayoutMask {
    #[must_use]
    pub fn contains(&self, x: u32, y: u32) -> bool {
        let Some(local_x) = x.checked_sub(self.x) else {
            return false;
        };
        let Some(local_y) = y.checked_sub(self.y) else {
            return false;
        };
        if local_x >= self.width || local_y >= self.height {
            return false;
        }
        self.pixels
            .get(local_y as usize * self.width as usize + local_x as usize)
            .is_some_and(|value| *value != 0)
    }
}

fn tensor_to_vec_f32(tensor: &Tensor) -> Result<Vec<f32>> {
    let tensor = tensor
        .to_device(Device::Cpu)
        .to_kind(Kind::Float)
        .contiguous();
    let length = tensor.numel();
    let mut values = vec![0.0; length];
    tensor.f_copy_data(&mut values, length)?;
    Ok(values)
}

fn tensor_to_vec_i64(tensor: &Tensor) -> Result<Vec<i64>> {
    let tensor = tensor
        .to_device(Device::Cpu)
        .to_kind(Kind::Int64)
        .contiguous();
    let length = tensor.numel();
    let mut values = vec![0; length];
    tensor.f_copy_data(&mut values, length)?;
    Ok(values)
}

fn tensor_to_vec_u8(tensor: &Tensor) -> Result<Vec<u8>> {
    let tensor = tensor
        .to_device(Device::Cpu)
        .to_kind(Kind::Uint8)
        .contiguous();
    let length = tensor.numel();
    let mut values = vec![0; length];
    tensor.f_copy_data(&mut values, length)?;
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use koharu_runtime::{Feature, Runtime};

    async fn preload_libtorch() -> Result<()> {
        Runtime::discover([Feature::Torch])?.initialize().await?;
        Ok(())
    }

    fn reference_masks(
        masks: &Tensor,
        width: u32,
        height: u32,
    ) -> Result<Vec<(u32, KoharuLayoutMask)>> {
        let mut resolved = Vec::new();
        for index in 0..masks.size()[0] {
            // Independently scan the original one-plane, full-page projection.
            // This reference has no batched reductions, extent reads or packing.
            let projected = masks
                .i(index)
                .unsqueeze(0)
                .unsqueeze(0)
                .upsample_bilinear2d([i64::from(height), i64::from(width)], false, None, None)
                .gt(0.0)
                .to_kind(Kind::Uint8);
            let full = tensor_to_vec_u8(&projected)?;
            let mut area = 0;
            let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
            for y in 0..height {
                for x in 0..width {
                    if full[y as usize * width as usize + x as usize] != 0 {
                        area += 1;
                        left = left.min(x);
                        top = top.min(y);
                        right = right.max(x + 1);
                        bottom = bottom.max(y + 1);
                    }
                }
            }
            if area == 0 {
                (left, top, right, bottom) = (0, 0, 0, 0);
            }
            let mut pixels = Vec::new();
            for y in top..bottom {
                for x in left..right {
                    pixels.push(full[y as usize * width as usize + x as usize] * 255);
                }
            }
            resolved.push((
                area,
                KoharuLayoutMask {
                    x: left,
                    y: top,
                    width: right - left,
                    height: bottom - top,
                    pixels,
                },
            ));
        }
        Ok(resolved)
    }

    fn assert_masks_equal(
        actual: &[(u32, KoharuLayoutMask)],
        expected: &[(u32, KoharuLayoutMask)],
    ) {
        assert_eq!(actual.len(), expected.len());
        for ((area, mask), (expected_area, expected_mask)) in actual.iter().zip(expected) {
            assert_eq!(area, expected_area);
            assert_eq!(
                [mask.x, mask.y, mask.width, mask.height],
                [
                    expected_mask.x,
                    expected_mask.y,
                    expected_mask.width,
                    expected_mask.height,
                ]
            );
            assert_eq!(mask.pixels, expected_mask.pixels);
        }
    }

    fn original_masks(
        masks: &Tensor,
        image_width: u32,
        image_height: u32,
    ) -> Result<Vec<(u32, KoharuLayoutMask)>> {
        // Retain the previous GPU reductions and two host reads per nonempty
        // instance as the timing baseline; the CPU scan reference above cannot
        // substitute for this because it downloads every full-page mask.
        let mut resolved = Vec::new();
        for index in 0..masks.size()[0] {
            let mask = masks
                .i(index)
                .unsqueeze(0)
                .unsqueeze(0)
                .upsample_bilinear2d(
                    [i64::from(image_height), i64::from(image_width)],
                    false,
                    None,
                    None,
                )
                .gt(0.0)
                .to_kind(Kind::Uint8);
            let rows = mask
                .count_nonzero_dim_intlist(&[0i64, 1, 3][..])
                .gt(0)
                .to_kind(Kind::Int64);
            let columns = mask
                .count_nonzero_dim_intlist(&[0i64, 1, 2][..])
                .gt(0)
                .to_kind(Kind::Int64);
            let occupied = Tensor::stack(
                &[
                    mask.count_nonzero(None),
                    columns.argmax(None, false),
                    rows.argmax(None, false),
                    i64::from(image_width) - columns.flip([0]).argmax(None, false),
                    i64::from(image_height) - rows.flip([0]).argmax(None, false),
                ],
                0,
            );
            let occupied = tensor_to_vec_i64(&occupied)?;
            let area = occupied[0].clamp(0, i64::from(u32::MAX)) as u32;
            let (x, y, right, bottom) = if area == 0 {
                (0, 0, 0, 0)
            } else {
                (
                    occupied[1] as u32,
                    occupied[2] as u32,
                    occupied[3] as u32,
                    occupied[4] as u32,
                )
            };
            let pixels = if right == x || bottom == y {
                Vec::new()
            } else {
                tensor_to_vec_u8(
                    &(mask.i((
                        0,
                        0,
                        i64::from(y)..i64::from(bottom),
                        i64::from(x)..i64::from(right),
                    )) * 255),
                )?
            };
            resolved.push((
                area,
                KoharuLayoutMask {
                    x,
                    y,
                    width: right - x,
                    height: bottom - y,
                    pixels,
                },
            ));
        }
        Ok(resolved)
    }

    #[tokio::test]
    #[ignore = "manually compare synthetic postprocessing on a CUDA device; no model is loaded"]
    async fn cuda_mask_chunks_match_original_and_report_timings() -> Result<()> {
        use std::{hint::black_box, time::Instant};

        use koharu_runtime::Backend;
        use koharu_torch::Cuda;

        let device = Runtime::discover([Feature::Torch])?.initialize().await?;
        ensure!(matches!(&device.backend, Backend::Cuda), "CUDA is required");
        ensure!(device.compute_capability() >= 80, "CUDA BF16 is required");
        let cuda_index = device.index as i64;
        let values = (0..16)
            .flat_map(|index| {
                (0..288).flat_map(move |y| {
                    (0..288).map(move |x| {
                        let inside = match index {
                            0 => true,
                            1 => (20..250).contains(&x) && (20..250).contains(&y),
                            15 => false,
                            _ => {
                                let left = (index % 4) * 60 + 8;
                                let top = (index / 4) * 60 + 8;
                                (left..left + 20 + index).contains(&x)
                                    && (top..top + 32 + index).contains(&y)
                            }
                        };
                        if inside { 2.0f32 } else { -2.0f32 }
                    })
                })
            })
            .collect::<Vec<_>>();
        let masks = Tensor::from_slice(&values)
            .view([16, 288, 288])
            .to_kind(Kind::BFloat16)
            .to_device(Device::Cuda(device.index));
        eprintln!(
            "synthetic mask postprocessing: device={} ({}), BF16, 16 masks, debug profile; no model, decode, typography, other pipeline stages or co-resident model load",
            device.name, device.description
        );
        for (width, height) in [(1152, 1700), (2000, 3000)] {
            // These untimed calls warm both paths and verify the complete
            // page-space output against the original implementation.
            let expected = original_masks(&masks, width, height)?;
            let actual = resolve_masks(&masks, width, height)?;
            assert_masks_equal(&actual, &expected);
            drop((expected, actual));
            let measure = |chunked| -> Result<_> {
                Cuda::synchronize(cuda_index);
                let start = Instant::now();
                let resolved = if chunked {
                    resolve_masks(&masks, width, height)?
                } else {
                    original_masks(&masks, width, height)?
                };
                Cuda::synchronize(cuda_index);
                let elapsed = start.elapsed();
                black_box(&resolved);
                Ok(elapsed)
            };
            let mut original = Vec::with_capacity(5);
            let mut chunked = Vec::with_capacity(5);
            for sample in 0..5 {
                let order = if sample % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                };
                for use_chunks in order {
                    let elapsed = measure(use_chunks)?;
                    if use_chunks {
                        chunked.push(elapsed);
                    } else {
                        original.push(elapsed);
                    }
                }
            }
            original.sort_unstable();
            chunked.sort_unstable();
            let chunk_size = mask_chunk_size(Kind::BFloat16, width, height);
            eprintln!(
                "{width}x{height}: chunk_size={chunk_size}, chunks={}, warmed median of 5: original={:.3}ms, chunked={:.3}ms; exact pixels/extents/area/order",
                16usize.div_ceil(chunk_size),
                original[2].as_secs_f64() * 1000.0,
                chunked[2].as_secs_f64() * 1000.0,
            );
        }
        Ok(())
    }

    #[test]
    fn mask_chunks_fit_the_work_budget_except_for_one_oversized_page() {
        assert_eq!(mask_chunk_size(Kind::Float, 5, 4), MAX_MASK_CHUNK);
        assert!(mask_chunk_size(Kind::BFloat16, 1152, 1152) > 1);
        assert!(mask_chunk_size(Kind::Float, 1152, 1152) < MAX_MASK_CHUNK);
        assert_eq!(mask_chunk_size(Kind::Float, 8192, 8192), 1);
        assert_eq!(mask_chunk_size(Kind::Float, u32::MAX, u32::MAX), 1);
    }

    #[tokio::test]
    async fn mask_chunks_match_full_page_reference_for_sparse_and_edge_masks() -> Result<()> {
        preload_libtorch().await?;
        let values = (0..17)
            .flat_map(|index| {
                (0..4).flat_map(move |y| {
                    (0..5).map(move |x| match index % 8 {
                        0 => -1.0,
                        1 => 0.0,
                        2 => 1.0,
                        3 => {
                            if x == 0 && y == 0 {
                                1.0
                            } else {
                                -1.0
                            }
                        }
                        4 => {
                            if x == 4 && y == 3 {
                                1.0
                            } else {
                                -1.0
                            }
                        }
                        5 => {
                            if x == y || x + y == 4 {
                                1.0
                            } else {
                                -1.0
                            }
                        }
                        6 => {
                            if x == 0 || x == 4 || y == 0 || y == 3 {
                                1.0
                            } else {
                                -1.0
                            }
                        }
                        _ => (x * 3 + y * 2) as f32 - 7.0,
                    })
                })
            })
            .collect::<Vec<_>>();
        for kind in [Kind::Float, Kind::BFloat16] {
            let masks = Tensor::from_slice(&values)
                .view([17, 4, 5])
                .transpose(1, 2)
                .to_kind(kind);
            for (width, height) in [(1, 1), (3, 11), (16, 9), (4, 5)] {
                let actual = resolve_masks(&masks, width, height)?;
                let expected = reference_masks(&masks, width, height)?;
                assert_masks_equal(&actual, &expected);
            }
        }
        let empty = Tensor::zeros([0, 4, 5], (Kind::Float, Device::Cpu));
        assert!(resolve_masks(&empty, 13, 7)?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn postprocessing_preserves_ranked_classes_and_repeated_query_masks() -> Result<()> {
        preload_libtorch().await?;
        let thresholds = KoharuLayoutThresholds {
            text: 0.5,
            onomatopoeia: 0.5,
            bubble: 0.5,
            panel: 0.5,
        };
        let processor = KoharuLayoutRFDetrImageProcessor {
            resolution: 1152,
            num_select: 12,
            class_names: ["text", "onomatopoeia", "bubble", "panel"]
                .map(str::to_owned)
                .into(),
            recommended_thresholds: thresholds,
        };
        let mut logits = vec![-20.0f32; 300 * 5];
        logits[0] = 3.0;
        logits[2] = 2.0;
        logits[8] = 4.0;
        logits[10] = 0.0; // Exactly the threshold is excluded.
        logits[19] = 10.0; // Background cannot create a detection.
        let mut boxes = vec![0.0f32; 300 * 4];
        boxes[..8].copy_from_slice(&[0.5, 0.4, 0.6, 0.2, 0.2, 0.7, 0.1, 0.1]);
        let masks = Tensor::zeros([1, 300, 288, 288], (Kind::Float, Device::Cpu));
        let _ = masks.i((0, 0, 0..144, 0..144)).fill_(2.0);
        let _ = masks.i((0, 1)).fill_(1.0);
        let output = Output {
            pred_logits: Tensor::from_slice(&logits).view([1, 300, 5]),
            pred_boxes: Tensor::from_slice(&boxes).view([1, 300, 4]),
            pred_masks: masks,
        };
        let actual = processor.postprocess(&output, 13, 7, thresholds)?;
        assert_eq!(actual.image_width, 13);
        assert_eq!(actual.image_height, 7);
        assert_eq!(
            actual
                .detections
                .iter()
                .map(|value| value.label_id)
                .collect::<Vec<_>>(),
            [3, 0, 2]
        );
        assert!(
            actual
                .detections
                .windows(2)
                .all(|pair| pair[0].score > pair[1].score)
        );
        assert_eq!(actual.detections[1].bbox, actual.detections[2].bbox);
        let selected = output
            .pred_masks
            .i(0)
            .index_select(0, &Tensor::from_slice(&[1i64, 0, 0]));
        let expected = reference_masks(&selected, 13, 7)?;
        let actual = actual
            .detections
            .into_iter()
            .map(|value| (value.area, value.mask))
            .collect::<Vec<_>>();
        assert_masks_equal(&actual, &expected);
        Ok(())
    }

    #[test]
    fn class_thresholds_must_be_probabilities() {
        let thresholds = KoharuLayoutThresholds {
            text: 0.25,
            onomatopoeia: 1.1,
            bubble: 0.5,
            panel: 0.5,
        };

        let error = validate_thresholds(thresholds).unwrap_err();
        assert!(error.to_string().contains("onomatopoeia"));
    }
}
