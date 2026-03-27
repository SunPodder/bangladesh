use crate::constants::{CHUNK_SIZE_METERS, DEFAULT_TERRAIN};
use crate::geometry::{Bounds, point_in_polygon, polygon_bounds};
use crate::terrain_types::TerrainPolygon;
use anyhow::Result;
use bangladesh::shared::world::TerrainKind;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashMap;

pub struct RasterChunkIndex {
    pub min_chunk_x: i32,
    pub min_chunk_y: i32,
    pub max_chunk_x: i32,
    pub max_chunk_y: i32,
    pub chunk_polygons: HashMap<(i32, i32), Vec<usize>>,
    polygon_bounds: Vec<Bounds>,
}

impl RasterChunkIndex {
    pub fn chunk_count(&self) -> usize {
        self.chunk_polygons.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunk_polygons.is_empty()
    }

    pub fn polygon_bounds(&self, polygon_idx: usize) -> Bounds {
        self.polygon_bounds[polygon_idx]
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
    let mut chunk_polygons: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    let mut polygon_bounds_cache = Vec::with_capacity(polygons.len());

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

        let polygon_min_chunk_x = (bounds.min_x / CHUNK_SIZE_METERS).floor() as i32;
        let polygon_max_chunk_x = (bounds.max_x / CHUNK_SIZE_METERS).floor() as i32;
        let polygon_min_chunk_y = (bounds.min_y / CHUNK_SIZE_METERS).floor() as i32;
        let polygon_max_chunk_y = (bounds.max_y / CHUNK_SIZE_METERS).floor() as i32;

        min_chunk_x = min_chunk_x.min(polygon_min_chunk_x);
        min_chunk_y = min_chunk_y.min(polygon_min_chunk_y);
        max_chunk_x = max_chunk_x.max(polygon_max_chunk_x);
        max_chunk_y = max_chunk_y.max(polygon_max_chunk_y);

        for chunk_y in polygon_min_chunk_y..=polygon_max_chunk_y {
            for chunk_x in polygon_min_chunk_x..=polygon_max_chunk_x {
                chunk_polygons
                    .entry((chunk_x, chunk_y))
                    .or_default()
                    .push(polygon_idx);
            }
        }

        progress.inc(1);
    }

    progress.finish_with_message("Polygon-to-chunk indexing complete");

    if polygons.is_empty() {
        return RasterChunkIndex {
            min_chunk_x: 0,
            min_chunk_y: 0,
            max_chunk_x: -1,
            max_chunk_y: -1,
            chunk_polygons,
            polygon_bounds: polygon_bounds_cache,
        };
    }

    RasterChunkIndex {
        min_chunk_x,
        min_chunk_y,
        max_chunk_x,
        max_chunk_y,
        chunk_polygons,
        polygon_bounds: polygon_bounds_cache,
    }
}

pub fn stream_rasterized_chunks<F>(
    polygons: &[TerrainPolygon],
    chunk_index: &RasterChunkIndex,
    cells_per_side: usize,
    emit_chunk: F,
) -> Result<usize>
where
    F: FnMut(i32, i32, &[u8]) -> Result<()>,
{
    let mut emit_chunk = emit_chunk;
    let mut chunk_keys = chunk_index
        .chunk_polygons
        .keys()
        .copied()
        .collect::<Vec<_>>();
    chunk_keys.sort_by_key(|(chunk_x, chunk_y)| (*chunk_y, *chunk_x));

    let progress = ProgressBar::new(chunk_keys.len() as u64);
    if let Ok(style) = ProgressStyle::with_template(
        "[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} chunks",
    ) {
        progress.set_style(style.progress_chars("##-"));
    }

    let default_code = DEFAULT_TERRAIN.code();
    let mut cells_buffer = vec![default_code; cells_per_side * cells_per_side];

    for (chunk_x, chunk_y) in chunk_keys {
        cells_buffer.fill(default_code);

        if let Some(polygon_indices) = chunk_index.chunk_polygons.get(&(chunk_x, chunk_y)) {
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

        emit_chunk(chunk_x, chunk_y, &cells_buffer)?;
        progress.inc(1);
    }

    progress.finish_with_message("Chunk rasterization stream complete");
    Ok(chunk_index.chunk_count())
}
