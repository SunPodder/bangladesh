use crate::constants::{CHUNK_SIZE_METERS, DEFAULT_TERRAIN};
use anyhow::{Context, Result, anyhow, ensure};
use bangladesh::shared::world::TerrainKind;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const LEVEL_RECORD_COORD_BYTES: usize = 8;
static LEVEL_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
pub struct PyramidLayout {
    pub playable_zoom_level: u8,
    pub playable_tile_offset_x: i32,
    pub playable_tile_offset_y: i32,
    pub playable_tile_size_m: f32,
}

#[derive(Debug)]
pub struct LevelSpoolWriter {
    path: PathBuf,
    file: File,
    cells_len: usize,
    record_count: usize,
}

#[derive(Debug)]
struct LevelTileRecord {
    tile_x: i32,
    tile_y: i32,
    cells: Vec<u8>,
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

impl LevelSpoolWriter {
    pub fn create(output_dir: &Path, region: &str, label: &str, zoom: u8, cells_per_side: usize) -> Result<Self> {
        let cells_len = cells_per_side
            .checked_mul(cells_per_side)
            .ok_or_else(|| anyhow!("cells per side overflows usize"))?;
        let path = build_temp_level_path(output_dir, region, label, zoom);
        let file = OpenOptions::new()
            .read(false)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .with_context(|| format!("failed to create temporary level spool at {}", path.display()))?;

        Ok(Self {
            path,
            file,
            cells_len,
            record_count: 0,
        })
    }

    pub fn append_tile(&mut self, tile_x: i32, tile_y: i32, cells: &[u8]) -> Result<()> {
        ensure!(
            cells.len() == self.cells_len,
            "tile cell payload size mismatch while writing level spool"
        );

        self.file
            .write_all(&tile_x.to_le_bytes())
            .context("failed to write level spool tile_x")?;
        self.file
            .write_all(&tile_y.to_le_bytes())
            .context("failed to write level spool tile_y")?;
        self.file
            .write_all(cells)
            .context("failed to write level spool cells")?;

        self.record_count += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Result<PathBuf> {
        self.file
            .flush()
            .context("failed to flush temporary level spool")?;
        Ok(self.path)
    }
}

pub fn build_parent_levels_from_spool<F>(
    base_level_path: &Path,
    output_dir: &Path,
    region: &str,
    cells_per_side: usize,
    playable_zoom_level: u8,
    mut emit_tile: F,
) -> Result<usize>
where
    F: FnMut(u8, i32, i32, Vec<u8>) -> Result<()>,
{
    if playable_zoom_level == 0 {
        return Ok(0);
    }

    let mut child_level_path = base_level_path.to_path_buf();
    let mut parent_tile_count = 0_usize;

    for parent_zoom in (0..playable_zoom_level).rev() {
        let (generated, parent_level_path) = reduce_child_level_to_parent(
            &child_level_path,
            output_dir,
            region,
            parent_zoom,
            cells_per_side,
            &mut emit_tile,
        )?;
        parent_tile_count += generated;

        let _ = std::fs::remove_file(&child_level_path);
        child_level_path = parent_level_path;
    }

    let _ = std::fs::remove_file(&child_level_path);
    Ok(parent_tile_count)
}

fn reduce_child_level_to_parent<F>(
    child_level_path: &Path,
    output_dir: &Path,
    region: &str,
    parent_zoom: u8,
    cells_per_side: usize,
    emit_tile: &mut F,
) -> Result<(usize, PathBuf)>
where
    F: FnMut(u8, i32, i32, Vec<u8>) -> Result<()>,
{
    let cells_len = cells_per_side
        .checked_mul(cells_per_side)
        .ok_or_else(|| anyhow!("cells per side overflows usize"))?;
    let record_size = (LEVEL_RECORD_COORD_BYTES + cells_len) as u64;

    let input_len = std::fs::metadata(child_level_path)
        .with_context(|| {
            format!(
                "failed to stat temporary child level spool {}",
                child_level_path.display()
            )
        })?
        .len();

    ensure!(
        input_len % record_size == 0,
        "temporary child level spool is corrupted: invalid record alignment"
    );

    let input_record_count = input_len / record_size;
    let mut child_file = File::open(child_level_path).with_context(|| {
        format!(
            "failed to open temporary child level spool {}",
            child_level_path.display()
        )
    })?;

    let mut parent_writer =
        LevelSpoolWriter::create(output_dir, region, "pyramid", parent_zoom, cells_per_side)?;

    let progress = ProgressBar::new(input_record_count);
    if let Ok(style) = ProgressStyle::with_template(
        "[{elapsed_precise}] {bar:40.green/blue} {pos}/{len} z{msg} records",
    ) {
        progress.set_style(style.progress_chars("##-"));
    }
    progress.set_message(parent_zoom.to_string());

    let mut generated = 0_usize;

    let mut current_row_y: Option<i32> = None;
    let mut current_row_tiles: HashMap<i32, Vec<u8>> = HashMap::new();

    let mut active_parent_y: Option<i32> = None;
    let mut even_row_tiles: HashMap<i32, Vec<u8>> = HashMap::new();
    let mut odd_row_tiles: HashMap<i32, Vec<u8>> = HashMap::new();

    while let Some(record) = read_level_record(&mut child_file, cells_len)? {
        progress.inc(1);

        match current_row_y {
            Some(row_y) if row_y != record.tile_y => {
                generated += process_child_row(
                    row_y,
                    std::mem::take(&mut current_row_tiles),
                    &mut active_parent_y,
                    &mut even_row_tiles,
                    &mut odd_row_tiles,
                    cells_per_side,
                    parent_zoom,
                    &mut parent_writer,
                    emit_tile,
                )?;
                current_row_y = Some(record.tile_y);
            }
            None => {
                current_row_y = Some(record.tile_y);
            }
            _ => {}
        }

        current_row_tiles.insert(record.tile_x, record.cells);
    }

    if let Some(row_y) = current_row_y {
        generated += process_child_row(
            row_y,
            std::mem::take(&mut current_row_tiles),
            &mut active_parent_y,
            &mut even_row_tiles,
            &mut odd_row_tiles,
            cells_per_side,
            parent_zoom,
            &mut parent_writer,
            emit_tile,
        )?;
    }

    if let Some(parent_y) = active_parent_y {
        generated += emit_parent_row(
            parent_y,
            &even_row_tiles,
            &odd_row_tiles,
            cells_per_side,
            parent_zoom,
            &mut parent_writer,
            emit_tile,
        )?;
    }

    progress.finish_with_message(format!("z{parent_zoom} reduction complete"));
    let parent_level_path = parent_writer.finish()?;

    Ok((generated, parent_level_path))
}

fn process_child_row<F>(
    row_y: i32,
    row_tiles: HashMap<i32, Vec<u8>>,
    active_parent_y: &mut Option<i32>,
    even_row_tiles: &mut HashMap<i32, Vec<u8>>,
    odd_row_tiles: &mut HashMap<i32, Vec<u8>>,
    cells_per_side: usize,
    parent_zoom: u8,
    parent_writer: &mut LevelSpoolWriter,
    emit_tile: &mut F,
) -> Result<usize>
where
    F: FnMut(u8, i32, i32, Vec<u8>) -> Result<()>,
{
    let mut generated = 0_usize;
    let row_parent_y = row_y.div_euclid(2);

    if active_parent_y.is_some() && *active_parent_y != Some(row_parent_y) {
        let previous_parent_y = active_parent_y
            .take()
            .ok_or_else(|| anyhow!("missing active parent row state"))?;
        generated += emit_parent_row(
            previous_parent_y,
            even_row_tiles,
            odd_row_tiles,
            cells_per_side,
            parent_zoom,
            parent_writer,
            emit_tile,
        )?;
        even_row_tiles.clear();
        odd_row_tiles.clear();
    }

    *active_parent_y = Some(row_parent_y);

    if row_y.rem_euclid(2) == 0 {
        *even_row_tiles = row_tiles;
    } else {
        *odd_row_tiles = row_tiles;
    }

    Ok(generated)
}

fn emit_parent_row<F>(
    parent_y: i32,
    even_row_tiles: &HashMap<i32, Vec<u8>>,
    odd_row_tiles: &HashMap<i32, Vec<u8>>,
    cells_per_side: usize,
    parent_zoom: u8,
    parent_writer: &mut LevelSpoolWriter,
    emit_tile: &mut F,
) -> Result<usize>
where
    F: FnMut(u8, i32, i32, Vec<u8>) -> Result<()>,
{
    if even_row_tiles.is_empty() && odd_row_tiles.is_empty() {
        return Ok(0);
    }

    let mut parent_xs = HashSet::new();
    for child_x in even_row_tiles.keys() {
        parent_xs.insert(child_x.div_euclid(2));
    }
    for child_x in odd_row_tiles.keys() {
        parent_xs.insert(child_x.div_euclid(2));
    }

    let mut sorted_parent_xs = parent_xs.into_iter().collect::<Vec<_>>();
    sorted_parent_xs.sort_unstable();

    let mut generated = 0_usize;
    for parent_x in sorted_parent_xs {
        let children = [
            even_row_tiles.get(&(parent_x * 2)).map(Vec::as_slice),
            even_row_tiles.get(&(parent_x * 2 + 1)).map(Vec::as_slice),
            odd_row_tiles.get(&(parent_x * 2)).map(Vec::as_slice),
            odd_row_tiles.get(&(parent_x * 2 + 1)).map(Vec::as_slice),
        ];

        let parent_cells = downsample_parent_tile(children, cells_per_side);
        parent_writer.append_tile(parent_x, parent_y, &parent_cells)?;
        emit_tile(parent_zoom, parent_x, parent_y, parent_cells)?;
        generated += 1;
    }

    Ok(generated)
}

fn read_level_record(file: &mut File, cells_len: usize) -> Result<Option<LevelTileRecord>> {
    let mut first = [0_u8; 1];
    let first_read = file
        .read(&mut first)
        .context("failed while reading level spool record")?;

    if first_read == 0 {
        return Ok(None);
    }

    let mut header = [0_u8; LEVEL_RECORD_COORD_BYTES];
    header[0] = first[0];
    file.read_exact(&mut header[1..])
        .context("failed to read level spool coordinate header")?;

    let tile_x = i32::from_le_bytes(
        header[0..4]
            .try_into()
            .map_err(|_| anyhow!("invalid level spool tile_x header bytes"))?,
    );
    let tile_y = i32::from_le_bytes(
        header[4..8]
            .try_into()
            .map_err(|_| anyhow!("invalid level spool tile_y header bytes"))?,
    );

    let mut cells = vec![0_u8; cells_len];
    file.read_exact(&mut cells)
        .context("failed to read level spool tile cells")?;

    Ok(Some(LevelTileRecord {
        tile_x,
        tile_y,
        cells,
    }))
}

fn build_temp_level_path(output_dir: &Path, region: &str, label: &str, zoom: u8) -> PathBuf {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0_u128, |duration| duration.as_nanos());
    let unique_counter = LEVEL_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);

    output_dir.join(format!(
        ".{region}.{label}.z{zoom}.{}.{}.{}.level.tmp",
        std::process::id(),
        unique_suffix,
        unique_counter
    ))
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
