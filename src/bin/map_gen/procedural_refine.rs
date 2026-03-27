use bangladesh::shared::world::TerrainKind;
use std::collections::VecDeque;

const CARDINAL_DIRS: [(isize, isize); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];

#[derive(Clone, Copy, Default)]
struct WaterComponentStats {
    size: usize,
    min_x: usize,
    min_y: usize,
    max_x: usize,
    max_y: usize,
    touches_boundary: bool,
}

fn index(x: usize, y: usize, side: usize) -> usize {
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
            Some(cells[index(nx as usize, ny as usize, side)] == TerrainKind::Road.code())
        })
        .filter(|is_road| *is_road)
        .count()
}

fn water_neighbor_count(cells: &[u8], side: usize, x: usize, y: usize) -> usize {
    CARDINAL_DIRS
        .iter()
        .filter_map(|(dx, dy)| {
            let nx = x as isize + dx;
            let ny = y as isize + dy;
            if !in_bounds(nx, ny, side) {
                return None;
            }
            Some(cells[index(nx as usize, ny as usize, side)] == TerrainKind::Water.code())
        })
        .filter(|is_water| *is_water)
        .count()
}

fn bridge_cardinal_gaps(cells: &mut [u8], side: usize, code: u8, passes: usize) {
    for _ in 0..passes {
        let mut fill_indices = Vec::new();

        for y in 1..(side.saturating_sub(1)) {
            for x in 1..(side.saturating_sub(1)) {
                let idx = index(x, y, side);
                if cells[idx] == code {
                    continue;
                }

                let up = cells[index(x, y - 1, side)] == code;
                let down = cells[index(x, y + 1, side)] == code;
                let left = cells[index(x - 1, y, side)] == code;
                let right = cells[index(x + 1, y, side)] == code;

                if (up && down) || (left && right) {
                    fill_indices.push(idx);
                }
            }
        }

        if fill_indices.is_empty() {
            break;
        }

        for idx in fill_indices {
            cells[idx] = code;
        }
    }
}

fn trim_short_dangling_roads(cells: &mut [u8], side: usize, max_stub_len: usize) {
    let road_code = TerrainKind::Road.code();
    let mut removal_mask = vec![false; cells.len()];

    for y in 0..side {
        for x in 0..side {
            let start_idx = index(x, y, side);
            if cells[start_idx] != road_code {
                continue;
            }
            if is_boundary_cell(x, y, side) {
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

                    if cells[index(next.0, next.1, side)] == road_code {
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
                    removal_mask[index(px, py, side)] = true;
                }
            }
        }
    }

    for (idx, should_remove) in removal_mask.into_iter().enumerate() {
        if should_remove {
            cells[idx] = TerrainKind::Grass.code();
        }
    }
}

fn collect_water_components(cells: &[u8], side: usize) -> Vec<WaterComponentStats> {
    let mut visited = vec![false; cells.len()];
    let mut components = Vec::new();
    let water_code = TerrainKind::Water.code();

    for y in 0..side {
        for x in 0..side {
            let root_idx = index(x, y, side);
            if visited[root_idx] || cells[root_idx] != water_code {
                continue;
            }

            let mut queue = VecDeque::new();
            queue.push_back((x, y));
            visited[root_idx] = true;

            let mut stats = WaterComponentStats {
                size: 0,
                min_x: x,
                min_y: y,
                max_x: x,
                max_y: y,
                touches_boundary: is_boundary_cell(x, y, side),
            };

            while let Some((cx, cy)) = queue.pop_front() {
                stats.size += 1;
                stats.min_x = stats.min_x.min(cx);
                stats.min_y = stats.min_y.min(cy);
                stats.max_x = stats.max_x.max(cx);
                stats.max_y = stats.max_y.max(cy);
                stats.touches_boundary |= is_boundary_cell(cx, cy, side);

                for (dx, dy) in CARDINAL_DIRS {
                    let nx = cx as isize + dx;
                    let ny = cy as isize + dy;
                    if !in_bounds(nx, ny, side) {
                        continue;
                    }

                    let nxu = nx as usize;
                    let nyu = ny as usize;
                    let nidx = index(nxu, nyu, side);

                    if visited[nidx] || cells[nidx] != water_code {
                        continue;
                    }

                    visited[nidx] = true;
                    queue.push_back((nxu, nyu));
                }
            }

            components.push(stats);
        }
    }

    components
}

fn has_river_like_water(cells: &[u8], side: usize) -> bool {
    collect_water_components(cells, side).iter().any(|component| {
        let width = component.max_x - component.min_x + 1;
        let height = component.max_y - component.min_y + 1;
        let major = width.max(height);
        let minor = width.min(height).max(1);
        let elongated = (major as f64) / (minor as f64) >= 2.3;

        component.size >= 48
            || (component.size >= 20 && elongated)
            || (component.size >= 24 && component.touches_boundary)
    })
}

fn fill_river_gaps(cells: &mut [u8], side: usize, passes: usize) {
    let water_code = TerrainKind::Water.code();
    let road_code = TerrainKind::Road.code();

    for _ in 0..passes {
        let mut fill_indices = Vec::new();

        for y in 1..(side.saturating_sub(1)) {
            for x in 1..(side.saturating_sub(1)) {
                let idx = index(x, y, side);
                if cells[idx] == water_code || cells[idx] == road_code {
                    continue;
                }

                let up = cells[index(x, y - 1, side)] == water_code;
                let down = cells[index(x, y + 1, side)] == water_code;
                let left = cells[index(x - 1, y, side)] == water_code;
                let right = cells[index(x + 1, y, side)] == water_code;

                let cardinal = [up, down, left, right]
                    .into_iter()
                    .filter(|b| *b)
                    .count();

                let diagonal = [
                    cells[index(x - 1, y - 1, side)] == water_code,
                    cells[index(x + 1, y - 1, side)] == water_code,
                    cells[index(x - 1, y + 1, side)] == water_code,
                    cells[index(x + 1, y + 1, side)] == water_code,
                ]
                .into_iter()
                .filter(|b| *b)
                .count();

                if (up && down) || (left && right) || (cardinal >= 3) || (cardinal >= 2 && diagonal >= 2) {
                    fill_indices.push(idx);
                }
            }
        }

        if fill_indices.is_empty() {
            break;
        }

        for idx in fill_indices {
            cells[idx] = water_code;
        }
    }
}

fn remove_road_islands(cells: &mut [u8], side: usize, max_island_cells: usize) {
    let road_code = TerrainKind::Road.code();
    let mut visited = vec![false; cells.len()];
    let mut remove = Vec::new();

    for y in 0..side {
        for x in 0..side {
            let root_idx = index(x, y, side);
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
                    let nidx = index(nxu, nyu, side);
                    if visited[nidx] || cells[nidx] != road_code {
                        continue;
                    }

                    visited[nidx] = true;
                    queue.push_back((nxu, nyu));
                }
            }

            if !touches_boundary && component.len() <= max_island_cells {
                remove.extend(component);
            }
        }
    }

    for (x, y) in remove {
        cells[index(x, y, side)] = TerrainKind::Grass.code();
    }
}

pub fn refine_chunk_cells(cells: &mut [u8], cells_per_side: usize) {
    if cells_per_side < 3 || cells.len() != cells_per_side * cells_per_side {
        return;
    }

    bridge_cardinal_gaps(cells, cells_per_side, TerrainKind::Road.code(), 1);
    trim_short_dangling_roads(cells, cells_per_side, 4);
    remove_road_islands(cells, cells_per_side, 6);

    let water_before = cells
        .iter()
        .copied()
        .filter(|code| *code == TerrainKind::Water.code())
        .count();

    if has_river_like_water(cells, cells_per_side) || water_before > (cells.len() / 16).max(8) {
        bridge_cardinal_gaps(cells, cells_per_side, TerrainKind::Water.code(), 1);
        fill_river_gaps(cells, cells_per_side, 1);
    }

    // Keep local water continuity stronger than roads when both are procedurally repaired.
    for y in 0..cells_per_side {
        for x in 0..cells_per_side {
            let idx = index(x, y, cells_per_side);
            if cells[idx] != TerrainKind::Road.code() {
                continue;
            }

            if water_neighbor_count(cells, cells_per_side, x, y) >= 3 {
                cells[idx] = TerrainKind::Water.code();
            }
        }
    }
}