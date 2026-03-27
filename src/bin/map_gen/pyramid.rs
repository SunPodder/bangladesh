use crate::constants::{CHUNK_SIZE_METERS, DEFAULT_TERRAIN};
use anyhow::{Result, anyhow, ensure};
use bangladesh::shared::world::TerrainKind;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Copy)]
pub struct PyramidLayout {
    pub playable_zoom_level: u8,
    pub playable_tile_offset_x: i32,
    pub playable_tile_offset_y: i32,
    pub playable_tile_size_m: f32,
}

#[derive(Debug)]
struct ParentLevelReducer {
    parent_zoom: u8,
    cells_per_side: usize,
    current_child_row_y: Option<i32>,
    current_row_tiles: HashMap<i32, Vec<u8>>,
    active_parent_y: Option<i32>,
    even_row_tiles: HashMap<i32, Vec<u8>>,
    odd_row_tiles: HashMap<i32, Vec<u8>>,
}

#[derive(Debug)]
pub struct PyramidStreamReducer {
    levels: Vec<ParentLevelReducer>,
}

fn ceil_log2(value: u32) -> u8 {
    if value <= 1 {
        return 0;
    }

    (u32::BITS - (value - 1).leading_zeros()) as u8
}

pub fn compute_pyramid_layout(
    min_chunk_x: i32,
    min_chunk_y: i32,
    max_chunk_x: i32,
    max_chunk_y: i32,
) -> Result<PyramidLayout> {
    ensure!(
        max_chunk_x >= min_chunk_x && max_chunk_y >= min_chunk_y,
        "invalid chunk bounds for playable layout"
    );

    let width = (max_chunk_x - min_chunk_x + 1) as u32;
    let height = (max_chunk_y - min_chunk_y + 1) as u32;
    let overview_zoom_levels = ceil_log2(width.max(height));

    let chunk_grid_side = 1_i32
        .checked_shl(overview_zoom_levels as u32)
        .ok_or_else(|| anyhow!("overview zoom level too large for i32 tile coordinates"))?;

    let chunk_offset_x = -min_chunk_x + (chunk_grid_side - width as i32) / 2;
    let chunk_offset_y = -min_chunk_y + (chunk_grid_side - height as i32) / 2;

    Ok(PyramidLayout {
        playable_zoom_level: overview_zoom_levels,
        playable_tile_offset_x: chunk_offset_x,
        playable_tile_offset_y: chunk_offset_y,
        playable_tile_size_m: CHUNK_SIZE_METERS as f32,
    })
}

impl ParentLevelReducer {
    fn new(parent_zoom: u8, cells_per_side: usize) -> Self {
        Self {
            parent_zoom,
            cells_per_side,
            current_child_row_y: None,
            current_row_tiles: HashMap::new(),
            active_parent_y: None,
            even_row_tiles: HashMap::new(),
            odd_row_tiles: HashMap::new(),
        }
    }

    fn ingest_child_tile(
        &mut self,
        child_tile_x: i32,
        child_tile_y: i32,
        child_cells: Vec<u8>,
    ) -> Result<Vec<(i32, i32, Vec<u8>)>> {
        let mut produced_tiles = Vec::new();

        if let Some(current_row_y) = self.current_child_row_y {
            ensure!(
                child_tile_y >= current_row_y,
                "child tiles must be streamed in non-decreasing y order"
            );

            if child_tile_y != current_row_y {
                produced_tiles.extend(self.flush_child_row()?);
            }
        }

        self.current_child_row_y = Some(child_tile_y);
        self.current_row_tiles.insert(child_tile_x, child_cells);

        Ok(produced_tiles)
    }

    fn finish(&mut self) -> Result<Vec<(i32, i32, Vec<u8>)>> {
        let mut produced_tiles = self.flush_child_row()?;

        if let Some(parent_y) = self.active_parent_y.take() {
            produced_tiles.extend(self.emit_parent_row(parent_y));
        }

        self.even_row_tiles.clear();
        self.odd_row_tiles.clear();

        Ok(produced_tiles)
    }

    fn flush_child_row(&mut self) -> Result<Vec<(i32, i32, Vec<u8>)>> {
        let Some(row_y) = self.current_child_row_y.take() else {
            return Ok(Vec::new());
        };

        let row_tiles = std::mem::take(&mut self.current_row_tiles);
        let row_parent_y = row_y.div_euclid(2);

        let mut produced_tiles = Vec::new();

        if self.active_parent_y.is_some() && self.active_parent_y != Some(row_parent_y) {
            let previous_parent_y = self
                .active_parent_y
                .take()
                .ok_or_else(|| anyhow!("missing active parent row state"))?;
            produced_tiles.extend(self.emit_parent_row(previous_parent_y));
            self.even_row_tiles.clear();
            self.odd_row_tiles.clear();
        }

        self.active_parent_y = Some(row_parent_y);
        if row_y.rem_euclid(2) == 0 {
            self.even_row_tiles = row_tiles;
        } else {
            self.odd_row_tiles = row_tiles;
        }

        Ok(produced_tiles)
    }

    fn emit_parent_row(&self, parent_y: i32) -> Vec<(i32, i32, Vec<u8>)> {
        if self.even_row_tiles.is_empty() && self.odd_row_tiles.is_empty() {
            return Vec::new();
        }

        let mut parent_xs = HashSet::new();
        for child_x in self.even_row_tiles.keys() {
            parent_xs.insert(child_x.div_euclid(2));
        }
        for child_x in self.odd_row_tiles.keys() {
            parent_xs.insert(child_x.div_euclid(2));
        }

        let mut sorted_parent_xs = parent_xs.into_iter().collect::<Vec<_>>();
        sorted_parent_xs.sort_unstable();

        let mut produced_tiles = Vec::with_capacity(sorted_parent_xs.len());
        for parent_x in sorted_parent_xs {
            let children = [
                self.even_row_tiles.get(&(parent_x * 2)).map(Vec::as_slice),
                self.even_row_tiles
                    .get(&(parent_x * 2 + 1))
                    .map(Vec::as_slice),
                self.odd_row_tiles.get(&(parent_x * 2)).map(Vec::as_slice),
                self.odd_row_tiles
                    .get(&(parent_x * 2 + 1))
                    .map(Vec::as_slice),
            ];

            let parent_cells = downsample_parent_tile(children, self.cells_per_side);
            produced_tiles.push((parent_x, parent_y, parent_cells));
        }

        produced_tiles
    }
}

impl PyramidStreamReducer {
    pub fn new(playable_zoom_level: u8, cells_per_side: usize) -> Result<Self> {
        ensure!(
            cells_per_side % 2 == 0,
            "cells_per_side must be even for 2x downsampling"
        );

        let mut levels = Vec::with_capacity(playable_zoom_level as usize);
        for parent_zoom in (0..playable_zoom_level).rev() {
            levels.push(ParentLevelReducer::new(parent_zoom, cells_per_side));
        }

        Ok(Self { levels })
    }

    pub fn push_playable_tile<F>(
        &mut self,
        playable_tile_x: i32,
        playable_tile_y: i32,
        playable_cells: Vec<u8>,
        emit_parent: &mut F,
    ) -> Result<usize>
    where
        F: FnMut(u8, i32, i32, &[u8]) -> Result<()>,
    {
        self.propagate_from_level(0, playable_tile_x, playable_tile_y, playable_cells, emit_parent)
    }

    pub fn finish<F>(&mut self, emit_parent: &mut F) -> Result<usize>
    where
        F: FnMut(u8, i32, i32, &[u8]) -> Result<()>,
    {
        let mut generated = 0_usize;

        for level_idx in 0..self.levels.len() {
            let parent_zoom = self.levels[level_idx].parent_zoom;
            let produced = self.levels[level_idx].finish()?;

            generated += produced.len();
            for (parent_x, parent_y, parent_cells) in produced {
                emit_parent(parent_zoom, parent_x, parent_y, &parent_cells)?;
                generated +=
                    self.propagate_from_level(level_idx + 1, parent_x, parent_y, parent_cells, emit_parent)?;
            }
        }

        Ok(generated)
    }

    fn propagate_from_level<F>(
        &mut self,
        start_level_idx: usize,
        child_tile_x: i32,
        child_tile_y: i32,
        child_cells: Vec<u8>,
        emit_parent: &mut F,
    ) -> Result<usize>
    where
        F: FnMut(u8, i32, i32, &[u8]) -> Result<()>,
    {
        if start_level_idx >= self.levels.len() {
            return Ok(0);
        }

        let mut generated = 0_usize;
        let mut queue = VecDeque::new();
        queue.push_back((start_level_idx, child_tile_x, child_tile_y, child_cells));

        while let Some((level_idx, tile_x, tile_y, cells)) = queue.pop_front() {
            if level_idx >= self.levels.len() {
                continue;
            }

            let parent_zoom = self.levels[level_idx].parent_zoom;
            let produced = self.levels[level_idx].ingest_child_tile(tile_x, tile_y, cells)?;

            generated += produced.len();
            for (parent_x, parent_y, parent_cells) in produced {
                emit_parent(parent_zoom, parent_x, parent_y, &parent_cells)?;
                queue.push_back((level_idx + 1, parent_x, parent_y, parent_cells));
            }
        }

        Ok(generated)
    }
}

fn lod_tie_break_priority(code: u8) -> u8 {
    match TerrainKind::from_code(code) {
        TerrainKind::Unknown => 0,
        // Keep water low in generic LOD ties so rivers do not expand into oceans.
        TerrainKind::Water => 1,
        TerrainKind::Grass => 2,
        TerrainKind::Farmland => 3,
        TerrainKind::Forest => 4,
        TerrainKind::Sand => 5,
        TerrainKind::Urban => 6,
    }
}

fn is_edge_connected_pair(mask: u8) -> bool {
    const TOP_ROW: u8 = (1 << 0) | (1 << 1);
    const BOTTOM_ROW: u8 = (1 << 2) | (1 << 3);
    const LEFT_COL: u8 = (1 << 0) | (1 << 2);
    const RIGHT_COL: u8 = (1 << 1) | (1 << 3);

    matches!(mask, TOP_ROW | BOTTOM_ROW | LEFT_COL | RIGHT_COL)
}

fn resolve_downsampled_cell(samples: [u8; 4]) -> u8 {
    let mut counts = [0_u8; 7];
    let mut water_mask = 0_u8;
    for (idx, code) in samples.into_iter().enumerate() {
        if code == TerrainKind::Water.code() {
            water_mask |= 1 << idx;
        }

        let bucket = usize::from(code.min(6));
        counts[bucket] += 1;
    }

    let water_count = counts[usize::from(TerrainKind::Water.code())];
    let max_count = counts.iter().copied().max().unwrap_or(0);

    // Preserve broad river channels at low zoom: if a 2-2 tie includes
    // edge-connected water samples, keep water for this parent cell.
    if water_count == 2 && max_count == 2 && is_edge_connected_pair(water_mask) {
        return TerrainKind::Water.code();
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

fn downsample_parent_tile(children: [Option<&[u8]>; 4], cells_per_side: usize) -> Vec<u8> {
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
    fn downsample_edge_connected_water_tie_is_preserved() {
        let resolved = resolve_downsampled_cell([
            TerrainKind::Water.code(),
            TerrainKind::Water.code(),
            TerrainKind::Grass.code(),
            TerrainKind::Grass.code(),
        ]);
        assert_eq!(resolved, TerrainKind::Water.code());
    }

    #[test]
    fn downsample_diagonal_water_tie_stays_land() {
        let resolved = resolve_downsampled_cell([
            TerrainKind::Water.code(),
            TerrainKind::Grass.code(),
            TerrainKind::Grass.code(),
            TerrainKind::Water.code(),
        ]);
        assert_eq!(resolved, TerrainKind::Grass.code());
    }
}
