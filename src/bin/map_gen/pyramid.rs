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

            let parent_cells = downsample_parent_tile(children, self.cells_per_side, self.parent_zoom);
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
        TerrainKind::Road => 1,
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

const CARDINAL_DIRS: [(isize, isize); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];
const DIAGONAL_DIRS: [(isize, isize); 4] = [(-1, -1), (1, -1), (-1, 1), (1, 1)];

fn cell_index(x: usize, y: usize, side: usize) -> usize {
    y * side + x
}

fn in_bounds(x: isize, y: isize, side: usize) -> bool {
    x >= 0 && y >= 0 && (x as usize) < side && (y as usize) < side
}

fn is_boundary_cell(x: usize, y: usize, side: usize) -> bool {
    x == 0 || y == 0 || x + 1 == side || y + 1 == side
}

fn road_neighbor_count(cells: &[u8], side: usize, x: usize, y: usize) -> usize {
    CARDINAL_DIRS
        .iter()
        .filter_map(|(dx, dy)| {
            let nx = x as isize + dx;
            let ny = y as isize + dy;
            if !in_bounds(nx, ny, side) {
                return None;
            }

            Some(cells[cell_index(nx as usize, ny as usize, side)] == TerrainKind::Road.code())
        })
        .filter(|is_road| *is_road)
        .count()
}

fn road_diagonal_neighbor_count(cells: &[u8], side: usize, x: usize, y: usize) -> usize {
    DIAGONAL_DIRS
        .iter()
        .filter_map(|(dx, dy)| {
            let nx = x as isize + dx;
            let ny = y as isize + dy;
            if !in_bounds(nx, ny, side) {
                return None;
            }

            Some(cells[cell_index(nx as usize, ny as usize, side)] == TerrainKind::Road.code())
        })
        .filter(|is_road| *is_road)
        .count()
}

fn dominant_non_road_neighbor(cells: &[u8], side: usize, x: usize, y: usize) -> u8 {
    let mut counts = [0_u8; 8];

    for ny in (y as isize - 1)..=(y as isize + 1) {
        for nx in (x as isize - 1)..=(x as isize + 1) {
            if !in_bounds(nx, ny, side) {
                continue;
            }
            if nx as usize == x && ny as usize == y {
                continue;
            }

            let neighbor = cells[cell_index(nx as usize, ny as usize, side)];
            if neighbor == TerrainKind::Road.code() {
                continue;
            }

            counts[usize::from(neighbor.min(7))] += 1;
        }
    }

    let mut winner = DEFAULT_TERRAIN.code();
    let mut winner_count = 0_u8;
    let mut winner_priority = lod_tie_break_priority(winner);

    for (code, count) in counts.iter().enumerate() {
        if *count == 0 {
            continue;
        }

        let candidate = code as u8;
        let candidate_priority = lod_tie_break_priority(candidate);
        if *count > winner_count || (*count == winner_count && candidate_priority > winner_priority) {
            winner = candidate;
            winner_count = *count;
            winner_priority = candidate_priority;
        }
    }

    winner
}

fn erode_dense_road_clusters(cells: &mut [u8], side: usize) {
    let road_code = TerrainKind::Road.code();
    let mut remove = Vec::new();

    for y in 1..(side.saturating_sub(1)) {
        for x in 1..(side.saturating_sub(1)) {
            let idx = cell_index(x, y, side);
            if cells[idx] != road_code || is_boundary_cell(x, y, side) {
                continue;
            }

            let cardinal = road_neighbor_count(cells, side, x, y);
            let diagonal = road_diagonal_neighbor_count(cells, side, x, y);

            // Interior blob cells tend to have high cardinal + diagonal road density.
            if cardinal >= 3 && diagonal >= 2 {
                remove.push((x, y));
            }
        }
    }

    for (x, y) in remove {
        let idx = cell_index(x, y, side);
        cells[idx] = dominant_non_road_neighbor(cells, side, x, y);
    }
}

fn remove_weak_road_cells(cells: &mut [u8], side: usize) {
    let road_code = TerrainKind::Road.code();
    let mut remove = Vec::new();

    for y in 0..side {
        for x in 0..side {
            let idx = cell_index(x, y, side);
            if cells[idx] != road_code || is_boundary_cell(x, y, side) {
                continue;
            }

            let cardinal = road_neighbor_count(cells, side, x, y);
            if cardinal == 0 {
                remove.push((x, y));
            }
        }
    }

    for (x, y) in remove {
        let idx = cell_index(x, y, side);
        cells[idx] = dominant_non_road_neighbor(cells, side, x, y);
    }
}

fn bridge_road_gaps(cells: &mut [u8], side: usize) {
    let road_code = TerrainKind::Road.code();
    let mut fill = Vec::new();

    for y in 1..(side.saturating_sub(1)) {
        for x in 1..(side.saturating_sub(1)) {
            let idx = cell_index(x, y, side);
            if cells[idx] == road_code {
                continue;
            }

            let up = cells[cell_index(x, y - 1, side)] == road_code;
            let down = cells[cell_index(x, y + 1, side)] == road_code;
            let left = cells[cell_index(x - 1, y, side)] == road_code;
            let right = cells[cell_index(x + 1, y, side)] == road_code;

            if (up && down) || (left && right) {
                fill.push(idx);
            }
        }
    }

    for idx in fill {
        cells[idx] = road_code;
    }
}

fn remove_small_road_components(cells: &mut [u8], side: usize, max_component_size: usize) {
    let road_code = TerrainKind::Road.code();
    let mut visited = vec![false; cells.len()];
    let mut remove = Vec::new();

    for y in 0..side {
        for x in 0..side {
            let root_idx = cell_index(x, y, side);
            if visited[root_idx] || cells[root_idx] != road_code {
                continue;
            }

            let mut queue = VecDeque::new();
            let mut component = Vec::new();
            let mut touches_boundary = false;

            visited[root_idx] = true;
            queue.push_back((x, y));

            while let Some((cx, cy)) = queue.pop_front() {
                component.push((cx, cy));
                touches_boundary |= is_boundary_cell(cx, cy, side);

                for (dx, dy) in CARDINAL_DIRS {
                    let nx = cx as isize + dx;
                    let ny = cy as isize + dy;
                    if !in_bounds(nx, ny, side) {
                        continue;
                    }

                    let nxu = nx as usize;
                    let nyu = ny as usize;
                    let nidx = cell_index(nxu, nyu, side);
                    if visited[nidx] || cells[nidx] != road_code {
                        continue;
                    }

                    visited[nidx] = true;
                    queue.push_back((nxu, nyu));
                }
            }

            if !touches_boundary && component.len() <= max_component_size {
                remove.extend(component);
            }
        }
    }

    for (x, y) in remove {
        let idx = cell_index(x, y, side);
        cells[idx] = dominant_non_road_neighbor(cells, side, x, y);
    }
}

fn trim_short_dangling_roads(cells: &mut [u8], side: usize, max_stub_len: usize) {
    let road_code = TerrainKind::Road.code();
    let mut remove_mask = vec![false; cells.len()];

    for y in 0..side {
        for x in 0..side {
            let start_idx = cell_index(x, y, side);
            if cells[start_idx] != road_code || is_boundary_cell(x, y, side) {
                continue;
            }
            if road_neighbor_count(cells, side, x, y) != 1 {
                continue;
            }

            let mut path = vec![(x, y)];
            let mut previous = (x, y);
            let mut current = (x, y);
            let mut reached_boundary = false;
            let mut reached_junction = false;
            let mut too_long = false;

            loop {
                let mut next_candidates = Vec::new();
                for (dx, dy) in CARDINAL_DIRS {
                    let nx = current.0 as isize + dx;
                    let ny = current.1 as isize + dy;
                    if !in_bounds(nx, ny, side) {
                        continue;
                    }

                    let next = (nx as usize, ny as usize);
                    if next == previous {
                        continue;
                    }

                    if cells[cell_index(next.0, next.1, side)] == road_code {
                        next_candidates.push(next);
                    }
                }

                if next_candidates.is_empty() {
                    break;
                }

                if next_candidates.len() > 1 {
                    reached_junction = true;
                    break;
                }

                let next = next_candidates[0];
                previous = current;
                current = next;

                if !path.contains(&current) {
                    path.push(current);
                } else {
                    too_long = true;
                    break;
                }

                if is_boundary_cell(current.0, current.1, side) {
                    reached_boundary = true;
                    break;
                }

                let degree = road_neighbor_count(cells, side, current.0, current.1);
                if degree >= 3 {
                    reached_junction = true;
                    break;
                }

                if path.len() > max_stub_len {
                    too_long = true;
                    break;
                }
            }

            if too_long || reached_boundary {
                continue;
            }

            if reached_junction || path.len() <= max_stub_len {
                for (px, py) in path {
                    remove_mask[cell_index(px, py, side)] = true;
                }
            }
        }
    }

    for (idx, should_remove) in remove_mask.into_iter().enumerate() {
        if should_remove {
            let x = idx % side;
            let y = idx / side;
            cells[idx] = dominant_non_road_neighbor(cells, side, x, y);
        }
    }
}

fn apply_low_lod_road_cleanup(cells: &mut [u8], side: usize, parent_zoom: u8) {
    if side < 3 || parent_zoom > 2 {
        return;
    }

    let cleanup_passes = match parent_zoom {
        0 => 3,
        1 => 2,
        _ => 1,
    };

    for _ in 0..cleanup_passes {
        erode_dense_road_clusters(cells, side);
        remove_weak_road_cells(cells, side);
        bridge_road_gaps(cells, side);
    }

    let max_component_size = match parent_zoom {
        0 => 8,
        1 => 7,
        _ => 6,
    };
    let max_stub_len = match parent_zoom {
        0 => 6,
        1 => 5,
        _ => 4,
    };

    remove_small_road_components(cells, side, max_component_size);
    trim_short_dangling_roads(cells, side, max_stub_len);
}

fn resolve_downsampled_cell(samples: [u8; 4]) -> u8 {
    let mut counts = [0_u8; 8];
    let mut water_mask = 0_u8;
    let mut road_mask = 0_u8;
    for (idx, code) in samples.into_iter().enumerate() {
        if code == TerrainKind::Water.code() {
            water_mask |= 1 << idx;
        } else if code == TerrainKind::Road.code() {
            road_mask |= 1 << idx;
        }

        let bucket = usize::from(code.min(7));
        counts[bucket] += 1;
    }

    let water_count = counts[usize::from(TerrainKind::Water.code())];
    let road_count = counts[usize::from(TerrainKind::Road.code())];
    let max_count = counts.iter().copied().max().unwrap_or(0);

    // Preserve broad river channels at low zoom: if a 2-2 tie includes
    // edge-connected water samples, keep water for this parent cell.
    if water_count == 2 && max_count == 2 && is_edge_connected_pair(water_mask) {
        return TerrainKind::Water.code();
    }

    // Preserve major road corridors at low zoom: if a 2-2 tie includes
    // edge-connected road samples, keep road for this parent cell.
    if road_count == 2 && max_count == 2 && is_edge_connected_pair(road_mask) {
        return TerrainKind::Road.code();
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

fn downsample_parent_tile(
    children: [Option<&[u8]>; 4],
    cells_per_side: usize,
    parent_zoom: u8,
) -> Vec<u8> {
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

    apply_low_lod_road_cleanup(&mut parent_cells, cells_per_side, parent_zoom);

    parent_cells
}

#[cfg(test)]
mod tests {
    use super::*;

    fn largest_road_component(cells: &[u8], side: usize) -> usize {
        let road_code = TerrainKind::Road.code();
        let mut visited = vec![false; cells.len()];
        let mut best = 0_usize;

        for y in 0..side {
            for x in 0..side {
                let root_idx = cell_index(x, y, side);
                if visited[root_idx] || cells[root_idx] != road_code {
                    continue;
                }

                let mut queue = VecDeque::new();
                let mut size = 0_usize;
                visited[root_idx] = true;
                queue.push_back((x, y));

                while let Some((cx, cy)) = queue.pop_front() {
                    size += 1;

                    for (dx, dy) in CARDINAL_DIRS {
                        let nx = cx as isize + dx;
                        let ny = cy as isize + dy;
                        if !in_bounds(nx, ny, side) {
                            continue;
                        }

                        let nxu = nx as usize;
                        let nyu = ny as usize;
                        let nidx = cell_index(nxu, nyu, side);
                        if visited[nidx] || cells[nidx] != road_code {
                            continue;
                        }

                        visited[nidx] = true;
                        queue.push_back((nxu, nyu));
                    }
                }

                best = best.max(size);
            }
        }

        best
    }

    fn road_count(cells: &[u8]) -> usize {
        cells
            .iter()
            .filter(|code| **code == TerrainKind::Road.code())
            .count()
    }

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

    #[test]
    fn downsample_edge_connected_road_tie_is_preserved() {
        let resolved = resolve_downsampled_cell([
            TerrainKind::Road.code(),
            TerrainKind::Road.code(),
            TerrainKind::Grass.code(),
            TerrainKind::Grass.code(),
        ]);
        assert_eq!(resolved, TerrainKind::Road.code());
    }

    #[test]
    fn downsample_diagonal_road_tie_stays_land() {
        let resolved = resolve_downsampled_cell([
            TerrainKind::Road.code(),
            TerrainKind::Grass.code(),
            TerrainKind::Grass.code(),
            TerrainKind::Road.code(),
        ]);
        assert_eq!(resolved, TerrainKind::Grass.code());
    }

    #[test]
    fn low_lod_cleanup_removes_orphan_road_cells() {
        let side = 16;
        let mut cells = vec![TerrainKind::Urban.code(); side * side];

        for x in 2..14 {
            cells[cell_index(x, 6, side)] = TerrainKind::Road.code();
        }

        let orphan_idx = cell_index(8, 12, side);
        cells[orphan_idx] = TerrainKind::Road.code();

        apply_low_lod_road_cleanup(&mut cells, side, 0);

        assert_ne!(cells[orphan_idx], TerrainKind::Road.code());
        assert!(largest_road_component(&cells, side) >= 8);
    }

    #[test]
    fn low_lod_cleanup_thins_dense_road_blocks() {
        let side = 18;
        let mut cells = vec![TerrainKind::Urban.code(); side * side];

        for y in 5..13 {
            for x in 5..13 {
                cells[cell_index(x, y, side)] = TerrainKind::Road.code();
            }
        }

        let before = road_count(&cells);
        apply_low_lod_road_cleanup(&mut cells, side, 0);
        let after = road_count(&cells);

        assert!(after < before - 10);
        assert_ne!(cells[cell_index(9, 9, side)], TerrainKind::Road.code());
    }

    #[test]
    fn low_lod_cleanup_preserves_major_corridor() {
        let side = 16;
        let mut cells = vec![TerrainKind::Grass.code(); side * side];

        for x in 1..15 {
            cells[cell_index(x, 8, side)] = TerrainKind::Road.code();
        }

        cells[cell_index(12, 3, side)] = TerrainKind::Road.code();
        cells[cell_index(13, 13, side)] = TerrainKind::Road.code();

        apply_low_lod_road_cleanup(&mut cells, side, 1);

        assert!(largest_road_component(&cells, side) >= 10);
        assert_ne!(cells[cell_index(12, 3, side)], TerrainKind::Road.code());
        assert_ne!(cells[cell_index(13, 13, side)], TerrainKind::Road.code());
    }
}
