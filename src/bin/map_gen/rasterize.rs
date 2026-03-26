use crate::constants::{CHUNK_SIZE_METERS, DEFAULT_TERRAIN};
use crate::geometry::{point_in_polygon, polygon_bounds};
use crate::terrain_types::TerrainPolygon;
use bangladesh::shared::world::TerrainKind;
use rayon::prelude::*;
use std::collections::HashMap;

fn paint_polygon_into_chunks(
    polygon: &TerrainPolygon,
    cells_per_side: usize,
    chunk_cells: &mut HashMap<(i32, i32), Vec<u8>>,
) {
    let bounds = polygon_bounds(&polygon.points);
    let cell_size = CHUNK_SIZE_METERS / cells_per_side as f64;

    let min_chunk_x = (bounds.min_x / CHUNK_SIZE_METERS).floor() as i32;
    let max_chunk_x = (bounds.max_x / CHUNK_SIZE_METERS).floor() as i32;
    let min_chunk_y = (bounds.min_y / CHUNK_SIZE_METERS).floor() as i32;
    let max_chunk_y = (bounds.max_y / CHUNK_SIZE_METERS).floor() as i32;

    let terrain_code = polygon.terrain.code();
    let terrain_priority = polygon.terrain.priority();

    for chunk_y in min_chunk_y..=max_chunk_y {
        for chunk_x in min_chunk_x..=max_chunk_x {
            let chunk_origin_x = f64::from(chunk_x) * CHUNK_SIZE_METERS;
            let chunk_origin_y = f64::from(chunk_y) * CHUNK_SIZE_METERS;

            let min_ix = (((bounds.min_x - chunk_origin_x) / cell_size).floor() as i32).max(0);
            let max_ix = (((bounds.max_x - chunk_origin_x) / cell_size).ceil() as i32)
                .min(cells_per_side as i32);
            let min_iy = (((bounds.min_y - chunk_origin_y) / cell_size).floor() as i32).max(0);
            let max_iy = (((bounds.max_y - chunk_origin_y) / cell_size).ceil() as i32)
                .min(cells_per_side as i32);

            if min_ix >= max_ix || min_iy >= max_iy {
                continue;
            }

            let cells = chunk_cells.entry((chunk_x, chunk_y)).or_insert_with(|| {
                vec![DEFAULT_TERRAIN.code(); cells_per_side * cells_per_side]
            });

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
    }
}

fn merge_chunk_maps(left: &mut HashMap<(i32, i32), Vec<u8>>, right: HashMap<(i32, i32), Vec<u8>>) {
    for (chunk_key, right_cells) in right {
        let Some(left_cells) = left.get_mut(&chunk_key) else {
            left.insert(chunk_key, right_cells);
            continue;
        };

        for (left_cell, right_cell) in left_cells.iter_mut().zip(right_cells.iter()) {
            let right_priority = TerrainKind::from_code(*right_cell).priority();
            let left_priority = TerrainKind::from_code(*left_cell).priority();
            if right_priority > left_priority {
                *left_cell = *right_cell;
            }
        }
    }
}

pub fn rasterize_polygons(
    polygons: &[TerrainPolygon],
    cells_per_side: usize,
) -> HashMap<(i32, i32), Vec<u8>> {
    polygons
        .par_iter()
        .fold(HashMap::new, |mut local_map, polygon| {
            paint_polygon_into_chunks(polygon, cells_per_side, &mut local_map);
            local_map
        })
        .reduce(HashMap::new, |mut left, right| {
            merge_chunk_maps(&mut left, right);
            left
        })
}
