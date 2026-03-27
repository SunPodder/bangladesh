use crate::constants::CHUNK_SIZE_METERS;
use crate::constants::DEFAULT_TERRAIN;
use anyhow::{Result, anyhow, ensure};
use bangladesh::shared::world::{TerrainKind, TerrainTile};
use std::collections::{HashMap, HashSet};

fn ceil_log2(value: u32) -> u8 {
    if value <= 1 {
        return 0;
    }

    (u32::BITS - (value - 1).leading_zeros()) as u8
}

fn lod_tie_break_priority(code: u8) -> u8 {
    match TerrainKind::from_code(code) {
        TerrainKind::Unknown => 0,
        // Keep water low in LOD ties so rivers don't expand into oceans.
        TerrainKind::Water => 1,
        TerrainKind::Grass => 2,
        TerrainKind::Farmland => 3,
        TerrainKind::Forest => 4,
        TerrainKind::Sand => 5,
        TerrainKind::Urban => 6,
    }
}

fn resolve_downsampled_cell(samples: [u8; 4]) -> u8 {
    let mut counts = [0_u8; 7];
    for code in samples {
        let bucket = usize::from(code.min(6));
        counts[bucket] += 1;
    }

    let mut winner = DEFAULT_TERRAIN.code();
    let mut winner_count = 0_u8;
    let mut winner_tie_priority = lod_tie_break_priority(winner);

    for (code, count) in counts.iter().enumerate() {
        if *count == 0 {
            continue;
        }

        let candidate = code as u8;
        let candidate_tie_priority = lod_tie_break_priority(candidate);
        if *count > winner_count
            || (*count == winner_count && candidate_tie_priority > winner_tie_priority)
        {
            winner = candidate;
            winner_count = *count;
            winner_tie_priority = candidate_tie_priority;
        }
    }

    winner
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

            let sample00 = child_cells[child_base_y * cells_per_side + child_base_x];
            let sample10 = child_cells[child_base_y * cells_per_side + (child_base_x + 1)];
            let sample01 = child_cells[(child_base_y + 1) * cells_per_side + child_base_x];
            let sample11 = child_cells[(child_base_y + 1) * cells_per_side + (child_base_x + 1)];
            let resolved = resolve_downsampled_cell([sample00, sample10, sample01, sample11]);

            let target_index = parent_y * cells_per_side + parent_x;
            parent_cells[target_index] = resolved;
        }
    }

    parent_cells
}

pub fn generate_tile_pyramid(
    base_chunks: HashMap<(i32, i32), Vec<u8>>,
    cells_per_side: usize,
) -> Result<(Vec<TerrainTile>, u8, i32, i32, f32)> {
    ensure!(
        !base_chunks.is_empty(),
        "cannot build tile pyramid from an empty playable chunk set"
    );
    ensure!(
        cells_per_side % 2 == 0,
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
    let overview_zoom_levels = ceil_log2(width.max(height));
    let playable_zoom_level = overview_zoom_levels;

    let chunk_grid_side = 1_i32
        .checked_shl(overview_zoom_levels as u32)
        .ok_or_else(|| anyhow!("overview zoom level too large for i32 tile coordinates"))?;

    let chunk_offset_x = -min_chunk_x + (chunk_grid_side - width as i32) / 2;
    let chunk_offset_y = -min_chunk_y + (chunk_grid_side - height as i32) / 2;

    let playable_tile_offset_x = chunk_offset_x;
    let playable_tile_offset_y = chunk_offset_y;

    let mut current_level = HashMap::with_capacity(base_chunks.len());
    for ((chunk_x, chunk_y), cells) in base_chunks {
        let tile_x = chunk_x + chunk_offset_x;
        let tile_y = chunk_y + chunk_offset_y;
        current_level.insert((tile_x, tile_y), cells);
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

            let parent_cells = downsample_parent_tile(children, cells_per_side);
            next_level.insert((parent_x, parent_y), parent_cells);
        }

        current_level = next_level;
        zoom -= 1;
    }

    all_tiles.sort_by_key(|tile| (tile.zoom, tile.tile_y, tile.tile_x));
    let playable_tile_size_m = CHUNK_SIZE_METERS as f32;
    Ok((
        all_tiles,
        playable_zoom_level,
        playable_tile_offset_x,
        playable_tile_offset_y,
        playable_tile_size_m,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsample_prefers_majority_over_priority() {
        let resolved = resolve_downsampled_cell([
            TerrainKind::Water.code(),
            TerrainKind::Grass.code(),
            TerrainKind::Grass.code(),
            TerrainKind::Grass.code(),
        ]);
        assert_eq!(resolved, TerrainKind::Grass.code());
    }

    #[test]
    fn downsample_tie_does_not_bias_toward_water() {
        let resolved = resolve_downsampled_cell([
            TerrainKind::Water.code(),
            TerrainKind::Water.code(),
            TerrainKind::Grass.code(),
            TerrainKind::Grass.code(),
        ]);
        assert_eq!(resolved, TerrainKind::Grass.code());
    }
}
