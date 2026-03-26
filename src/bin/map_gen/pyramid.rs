use crate::constants::{CELLS_PER_SIDE, DEFAULT_TERRAIN};
use anyhow::{Result, anyhow, ensure};
use bangladesh::shared::world::{TerrainKind, TerrainTile};
use std::collections::{HashMap, HashSet};

fn ceil_log2(value: u32) -> u8 {
    if value <= 1 {
        return 0;
    }

    (u32::BITS - (value - 1).leading_zeros()) as u8
}

fn merge_terrain_code_with_priority(current: u8, candidate: u8) -> u8 {
    let current_priority = TerrainKind::from_code(current).priority();
    let candidate_priority = TerrainKind::from_code(candidate).priority();

    if candidate_priority > current_priority {
        candidate
    } else {
        current
    }
}

fn downsample_parent_tile(children: [Option<&Vec<u8>>; 4], cells_per_side: usize) -> Vec<u8> {
    let mut parent_cells = vec![DEFAULT_TERRAIN.code(); cells_per_side * cells_per_side];
    let child_half_side = cells_per_side / 2;

    for parent_y in 0..cells_per_side {
        for parent_x in 0..cells_per_side {
            let quadrant_x = usize::from(parent_x >= child_half_side);
            let quadrant_y = usize::from(parent_y >= child_half_side);
            let child_index = quadrant_y * 2 + quadrant_x;

            let Some(child_cells) = children[child_index] else {
                continue;
            };

            let child_base_x = (parent_x % child_half_side) * 2;
            let child_base_y = (parent_y % child_half_side) * 2;

            let mut resolved = DEFAULT_TERRAIN.code();
            for sample_y in 0..2 {
                for sample_x in 0..2 {
                    let source_index =
                        (child_base_y + sample_y) * cells_per_side + (child_base_x + sample_x);
                    resolved =
                        merge_terrain_code_with_priority(resolved, child_cells[source_index]);
                }
            }

            let target_index = parent_y * cells_per_side + parent_x;
            parent_cells[target_index] = resolved;
        }
    }

    parent_cells
}

pub fn generate_tile_pyramid(
    base_chunks: HashMap<(i32, i32), Vec<u8>>,
) -> Result<(Vec<TerrainTile>, u8, i32, i32)> {
    ensure!(
        !base_chunks.is_empty(),
        "cannot build tile pyramid from an empty playable chunk set"
    );
    ensure!(
        CELLS_PER_SIDE % 2 == 0,
        "cells per side must be even for 2x downsampling"
    );

    let mut min_chunk_x = i32::MAX;
    let mut min_chunk_y = i32::MAX;
    let mut max_chunk_x = i32::MIN;
    let mut max_chunk_y = i32::MIN;

    for &(chunk_x, chunk_y) in base_chunks.keys() {
        min_chunk_x = min_chunk_x.min(chunk_x);
        min_chunk_y = min_chunk_y.min(chunk_y);
        max_chunk_x = max_chunk_x.max(chunk_x);
        max_chunk_y = max_chunk_y.max(chunk_y);
    }

    let width = (max_chunk_x - min_chunk_x + 1) as u32;
    let height = (max_chunk_y - min_chunk_y + 1) as u32;
    let playable_zoom_level = ceil_log2(width.max(height));

    let grid_side = 1_i32
        .checked_shl(playable_zoom_level as u32)
        .ok_or_else(|| anyhow!("playable zoom level too large for i32 tile coordinates"))?;

    let playable_tile_offset_x = -min_chunk_x + (grid_side - width as i32) / 2;
    let playable_tile_offset_y = -min_chunk_y + (grid_side - height as i32) / 2;

    let mut current_level = HashMap::with_capacity(base_chunks.len());
    for ((chunk_x, chunk_y), cells) in base_chunks {
        current_level.insert(
            (chunk_x + playable_tile_offset_x, chunk_y + playable_tile_offset_y),
            cells,
        );
    }

    let mut all_tiles = Vec::new();
    let mut zoom = playable_zoom_level;

    loop {
        for ((tile_x, tile_y), cells) in &current_level {
            all_tiles.push(TerrainTile {
                zoom,
                tile_x: *tile_x,
                tile_y: *tile_y,
                cells: cells.clone(),
            });
        }

        if zoom == 0 {
            break;
        }

        let mut parent_keys = HashSet::with_capacity((current_level.len() / 4).max(1));
        for &(tile_x, tile_y) in current_level.keys() {
            parent_keys.insert((tile_x.div_euclid(2), tile_y.div_euclid(2)));
        }

        let mut next_level = HashMap::with_capacity(parent_keys.len());
        for (parent_x, parent_y) in parent_keys {
            let children = [
                current_level.get(&(parent_x * 2, parent_y * 2)),
                current_level.get(&(parent_x * 2 + 1, parent_y * 2)),
                current_level.get(&(parent_x * 2, parent_y * 2 + 1)),
                current_level.get(&(parent_x * 2 + 1, parent_y * 2 + 1)),
            ];

            let parent_cells = downsample_parent_tile(children, CELLS_PER_SIDE);
            next_level.insert((parent_x, parent_y), parent_cells);
        }

        current_level = next_level;
        zoom -= 1;
    }

    all_tiles.sort_by_key(|tile| (tile.zoom, tile.tile_y, tile.tile_x));
    Ok((
        all_tiles,
        playable_zoom_level,
        playable_tile_offset_x,
        playable_tile_offset_y,
    ))
}
