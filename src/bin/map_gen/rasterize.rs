use crate::constants::{CHUNK_SIZE_METERS, DEFAULT_TERRAIN};
use crate::geometry::{Bounds, point_in_polygon, polygon_bounds};
use crate::terrain_types::TerrainPolygon;
use anyhow::{Result, ensure};
use bangladesh::shared::world::TerrainKind;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub struct PolygonChunkBounds {
    pub min_chunk_x: i32,
    pub min_chunk_y: i32,
    pub max_chunk_x: i32,
    pub max_chunk_y: i32,
}

impl PolygonChunkBounds {
    fn intersects_row_window(self, window_min_y: i32, window_max_y: i32) -> bool {
        self.max_chunk_y >= window_min_y && self.min_chunk_y <= window_max_y
    }
}

pub struct RasterChunkIndex {
    pub min_chunk_x: i32,
    pub min_chunk_y: i32,
    pub max_chunk_x: i32,
    pub max_chunk_y: i32,
    polygon_bounds: Vec<Bounds>,
    polygon_chunk_bounds: Vec<PolygonChunkBounds>,
}

impl RasterChunkIndex {
    pub fn chunk_span_width(&self) -> usize {
        (self.max_chunk_x - self.min_chunk_x + 1).max(0) as usize
    }

    pub fn chunk_span_height(&self) -> usize {
        (self.max_chunk_y - self.min_chunk_y + 1).max(0) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.polygon_bounds.is_empty()
    }

    pub fn polygon_bounds(&self, polygon_idx: usize) -> Bounds {
        self.polygon_bounds[polygon_idx]
    }

    pub fn polygon_chunk_bounds(&self, polygon_idx: usize) -> PolygonChunkBounds {
        self.polygon_chunk_bounds[polygon_idx]
    }
}

fn paint_polygon_into_chunk(
    polygon: &TerrainPolygon,
    bounds: Bounds,
    chunk_x: i32,
    chunk_y: i32,
    cells_per_side: usize,
    cells: &mut [u8],
) {
    let cell_size = CHUNK_SIZE_METERS / cells_per_side as f64;

    let terrain_code = polygon.terrain.code();
    let terrain_priority = polygon.terrain.priority();

    let chunk_origin_x = f64::from(chunk_x) * CHUNK_SIZE_METERS;
    let chunk_origin_y = f64::from(chunk_y) * CHUNK_SIZE_METERS;

    let min_ix = (((bounds.min_x - chunk_origin_x) / cell_size).floor() as i32).max(0);
    let max_ix = (((bounds.max_x - chunk_origin_x) / cell_size).ceil() as i32)
        .min(cells_per_side as i32);
    let min_iy = (((bounds.min_y - chunk_origin_y) / cell_size).floor() as i32).max(0);
    let max_iy = (((bounds.max_y - chunk_origin_y) / cell_size).ceil() as i32)
        .min(cells_per_side as i32);

    if min_ix >= max_ix || min_iy >= max_iy {
        return;
    }

    for iy in min_iy..max_iy {
        for ix in min_ix..max_ix {
            let world_x = chunk_origin_x + (f64::from(ix) + 0.5) * cell_size;
            let world_y = chunk_origin_y + (f64::from(iy) + 0.5) * cell_size;

            if !point_in_polygon([world_x, world_y], &polygon.points) {
                continue;
            }

            let index = (iy as usize) * cells_per_side + (ix as usize);
            let existing_priority = TerrainKind::from_code(cells[index]).priority();
            if terrain_priority >= existing_priority {
                cells[index] = terrain_code;
            }
        }
    }
}

pub fn build_chunk_polygon_index(polygons: &[TerrainPolygon]) -> RasterChunkIndex {
    let mut polygon_bounds_cache = Vec::with_capacity(polygons.len());
    let mut polygon_chunk_bounds = Vec::with_capacity(polygons.len());

    let mut min_chunk_x = i32::MAX;
    let mut min_chunk_y = i32::MAX;
    let mut max_chunk_x = i32::MIN;
    let mut max_chunk_y = i32::MIN;

    let progress = ProgressBar::new(polygons.len() as u64);
    if let Ok(style) =
        ProgressStyle::with_template("[{elapsed_precise}] {bar:40.yellow/blue} {pos}/{len} indexed")
    {
        progress.set_style(style.progress_chars("##-"));
    }

    for (polygon_idx, polygon) in polygons.iter().enumerate() {
        let bounds = polygon_bounds(&polygon.points);
        polygon_bounds_cache.push(bounds);

        let chunk_bounds = PolygonChunkBounds {
            min_chunk_x: (bounds.min_x / CHUNK_SIZE_METERS).floor() as i32,
            max_chunk_x: (bounds.max_x / CHUNK_SIZE_METERS).floor() as i32,
            min_chunk_y: (bounds.min_y / CHUNK_SIZE_METERS).floor() as i32,
            max_chunk_y: (bounds.max_y / CHUNK_SIZE_METERS).floor() as i32,
        };
        polygon_chunk_bounds.push(chunk_bounds);

        min_chunk_x = min_chunk_x.min(chunk_bounds.min_chunk_x);
        min_chunk_y = min_chunk_y.min(chunk_bounds.min_chunk_y);
        max_chunk_x = max_chunk_x.max(chunk_bounds.max_chunk_x);
        max_chunk_y = max_chunk_y.max(chunk_bounds.max_chunk_y);

        let _ = polygon_idx;
        progress.inc(1);
    }

    progress.finish_with_message("Polygon-to-chunk indexing complete");

    if polygons.is_empty() {
        return RasterChunkIndex {
            min_chunk_x: 0,
            min_chunk_y: 0,
            max_chunk_x: -1,
            max_chunk_y: -1,
            polygon_bounds: polygon_bounds_cache,
            polygon_chunk_bounds,
        };
    }

    RasterChunkIndex {
        min_chunk_x,
        min_chunk_y,
        max_chunk_x,
        max_chunk_y,
        polygon_bounds: polygon_bounds_cache,
        polygon_chunk_bounds,
    }
}

fn estimate_window_rows(
    cells_per_side: usize,
    span_width: usize,
    raster_memory_budget_bytes: u64,
) -> usize {
    let cells_per_chunk = cells_per_side.saturating_mul(cells_per_side).max(1);
    let bytes_per_chunk = cells_per_chunk as u64;
    let target_chunk_budget = raster_memory_budget_bytes
        .saturating_div(bytes_per_chunk.saturating_mul(6).max(1))
        .max(1024);
    let width = span_width.max(1) as u64;
    let estimated_rows = target_chunk_budget / width;

    estimated_rows.clamp(8, 256) as usize
}

pub fn stream_rasterized_chunks<F>(
    polygons: &[TerrainPolygon],
    chunk_index: &RasterChunkIndex,
    cells_per_side: usize,
    raster_memory_budget_bytes: u64,
    emit_chunk: F,
) -> Result<usize>
where
    F: FnMut(i32, i32, Vec<u8>) -> Result<()>,
{
    ensure!(
        raster_memory_budget_bytes > 0,
        "raster memory budget must be greater than zero"
    );

    let mut emit_chunk = emit_chunk;

    if chunk_index.is_empty() {
        return Ok(0);
    }

    let span_width = chunk_index.chunk_span_width();
    let span_height = chunk_index.chunk_span_height();
    let window_rows = estimate_window_rows(cells_per_side, span_width, raster_memory_budget_bytes);
    let total_windows = span_height.div_ceil(window_rows);

    let progress = ProgressBar::new(total_windows as u64);
    if let Ok(style) = ProgressStyle::with_template(
        "[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} windows ({msg})",
    ) {
        progress.set_style(style.progress_chars("##-"));
    }
    progress.set_message("0 chunks");

    let default_code = DEFAULT_TERRAIN.code();
    let cells_per_chunk = cells_per_side * cells_per_side;

    let mut polygons_by_min_y = (0..polygons.len()).collect::<Vec<_>>();
    polygons_by_min_y.sort_unstable_by_key(|polygon_idx| {
        chunk_index.polygon_chunk_bounds(*polygon_idx).min_chunk_y
    });

    let mut next_polygon_cursor = 0_usize;
    let mut active_polygons = Vec::new();

    let worker_count = rayon::current_num_threads().max(1);
    let batch_size = (worker_count * 4).max(1);

    let mut emitted_chunks = 0_usize;

    let mut window_min_y = chunk_index.min_chunk_y;
    while window_min_y <= chunk_index.max_chunk_y {
        let window_max_y = (window_min_y + window_rows as i32 - 1).min(chunk_index.max_chunk_y);

        while next_polygon_cursor < polygons_by_min_y.len() {
            let polygon_idx = polygons_by_min_y[next_polygon_cursor];
            if chunk_index.polygon_chunk_bounds(polygon_idx).min_chunk_y > window_max_y {
                break;
            }
            active_polygons.push(polygon_idx);
            next_polygon_cursor += 1;
        }

        active_polygons.retain(|polygon_idx| {
            chunk_index
                .polygon_chunk_bounds(*polygon_idx)
                .max_chunk_y
                >= window_min_y
        });

        let mut local_chunk_polygons: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for polygon_idx in active_polygons.iter().copied() {
            let polygon_chunk_bounds = chunk_index.polygon_chunk_bounds(polygon_idx);
            if !polygon_chunk_bounds.intersects_row_window(window_min_y, window_max_y) {
                continue;
            }

            let min_y = polygon_chunk_bounds.min_chunk_y.max(window_min_y);
            let max_y = polygon_chunk_bounds.max_chunk_y.min(window_max_y);

            for chunk_y in min_y..=max_y {
                for chunk_x in polygon_chunk_bounds.min_chunk_x..=polygon_chunk_bounds.max_chunk_x {
                    local_chunk_polygons
                        .entry((chunk_x, chunk_y))
                        .or_default()
                        .push(polygon_idx);
                }
            }
        }

        let mut chunk_keys = local_chunk_polygons.keys().copied().collect::<Vec<_>>();
        chunk_keys.sort_by_key(|(chunk_x, chunk_y)| (*chunk_y, *chunk_x));

        for chunk_batch in chunk_keys.chunks(batch_size) {
            let rasterized_batch = chunk_batch
                .par_iter()
                .copied()
                .map(|(chunk_x, chunk_y)| {
                    let mut cells_buffer = vec![default_code; cells_per_chunk];

                    if let Some(polygon_indices) = local_chunk_polygons.get(&(chunk_x, chunk_y)) {
                        for polygon_idx in polygon_indices {
                            let polygon = &polygons[*polygon_idx];
                            let bounds = chunk_index.polygon_bounds(*polygon_idx);
                            paint_polygon_into_chunk(
                                polygon,
                                bounds,
                                chunk_x,
                                chunk_y,
                                cells_per_side,
                                &mut cells_buffer,
                            );
                        }
                    }

                    (chunk_x, chunk_y, cells_buffer)
                })
                .collect::<Vec<_>>();

            for (chunk_x, chunk_y, cells) in rasterized_batch {
                emit_chunk(chunk_x, chunk_y, cells)?;
                emitted_chunks += 1;
            }
        }

        progress.inc(1);
        progress.set_message(format!("{} chunks", emitted_chunks));

        window_min_y = window_max_y + 1;
    }

    progress.finish_with_message("Chunk rasterization stream complete");
    Ok(emitted_chunks)
}
