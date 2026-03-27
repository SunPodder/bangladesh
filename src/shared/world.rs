use anyhow::{Context, Result, anyhow, ensure};
use indicatif::{ProgressBar, ProgressStyle};
use rkyv::{Archive, Deserialize, Serialize, access, rancor::Error as RkyvError, to_bytes};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const WORLD_MAGIC: &[u8; 8] = b"BDWORLD1";
pub const WORLD_VERSION: u32 = 2;
const WORLD_HEADER_SIZE: usize = 8 + 4 + 8;
const MAP_ASSETS_DIR: &str = "assets/map";
static WORLD_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[repr(u8)]
#[derive(
    Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub enum TerrainKind {
    Unknown = 0,
    Water = 1,
    Grass = 2,
    Forest = 3,
    Urban = 4,
    Farmland = 5,
    Sand = 6,
}

impl TerrainKind {
    pub fn from_code(value: u8) -> Self {
        match value {
            1 => Self::Water,
            2 => Self::Grass,
            3 => Self::Forest,
            4 => Self::Urban,
            5 => Self::Farmland,
            6 => Self::Sand,
            _ => Self::Unknown,
        }
    }

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn priority(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Grass => 1,
            Self::Farmland => 2,
            Self::Forest => 3,
            Self::Sand => 4,
            Self::Urban => 5,
            Self::Water => 6,
        }
    }
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
pub struct TerrainTile {
    pub zoom: u8,
    pub tile_x: i32,
    pub tile_y: i32,
    pub cells: Vec<u8>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
pub struct TileMetadata {
    pub zoom: u8,
    pub tile_x: i32,
    pub tile_y: i32,
    pub byte_offset: u64,
    pub byte_len: u32,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
pub struct WorldMetadata {
    pub region: String,
    pub source_pbf: String,
    pub generated_unix_seconds: u64,
    pub chunk_size_m: f32,
    pub cells_per_side: u16,
    pub playable_zoom_level: u8,
    pub playable_tile_offset_x: i32,
    pub playable_tile_offset_y: i32,
    pub mercator_origin_x_m: f64,
    pub mercator_origin_y_m: f64,
    pub local_bounds_min_x: f32,
    pub local_bounds_min_y: f32,
    pub local_bounds_max_x: f32,
    pub local_bounds_max_y: f32,
    pub tile_count: u32,
    pub tiles: Vec<TileMetadata>,
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkLocation {
    pub byte_offset: u64,
    pub byte_len: u32,
}

#[derive(Debug, Clone)]
pub struct WorldIndex {
    pub region: String,
    pub chunk_size_m: f32,
    pub cells_per_side: u16,
    pub playable_zoom_level: u8,
    pub playable_tile_offset_x: i32,
    pub playable_tile_offset_y: i32,
    pub local_bounds_min_x: f32,
    pub local_bounds_min_y: f32,
    pub local_bounds_max_x: f32,
    pub local_bounds_max_y: f32,
    pub tiles: HashMap<(u8, i32, i32), ChunkLocation>,
}

#[derive(Debug)]
pub struct WorldStreamReader {
    pub index: WorldIndex,
    file: File,
}

#[derive(Debug)]
pub struct WorldWriter {
    temp_tile_data_path: PathBuf,
    temp_tile_data_file: File,
    tile_entries: Vec<TileMetadata>,
    temp_tile_data_len: u64,
}

pub fn map_assets_path() -> PathBuf {
    Path::new(MAP_ASSETS_DIR).to_path_buf()
}

pub fn world_output_path(region: &str) -> PathBuf {
    map_assets_path().join(format!("{region}.world"))
}

pub fn write_world_file(
    path: &Path,
    metadata: WorldMetadata,
    tiles: Vec<TerrainTile>,
) -> Result<()> {
    let mut writer = WorldWriter::new(path)?;
    for tile in tiles {
        writer.write_tile(tile.zoom, tile.tile_x, tile.tile_y, tile.cells)?;
    }
    writer.finish(path, metadata)
}

impl WorldWriter {
    pub fn new(output_path: &Path) -> Result<Self> {
        let temp_tile_data_path = build_temp_tile_data_path(output_path);
        let temp_tile_data_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_tile_data_path)
            .with_context(|| {
                format!(
                    "failed to create temporary world tile data file at {}",
                    temp_tile_data_path.display()
                )
            })?;

        Ok(Self {
            temp_tile_data_path,
            temp_tile_data_file,
            tile_entries: Vec::new(),
            temp_tile_data_len: 0,
        })
    }

    pub fn write_tile(&mut self, zoom: u8, tile_x: i32, tile_y: i32, cells: Vec<u8>) -> Result<()> {
        let tile = TerrainTile {
            zoom,
            tile_x,
            tile_y,
            cells,
        };

        let bytes = to_bytes::<RkyvError>(&tile)
            .map_err(|err| anyhow!("failed to serialize terrain tile: {err}"))?;
        ensure!(
            u32::try_from(bytes.len()).is_ok(),
            "serialized terrain tile is too large"
        );

        self.temp_tile_data_file
            .write_all(&bytes)
            .context("failed to append serialized tile bytes to temporary world data")?;

        self.tile_entries.push(TileMetadata {
            zoom,
            tile_x,
            tile_y,
            byte_offset: self.temp_tile_data_len,
            byte_len: bytes.len() as u32,
        });
        self.temp_tile_data_len += bytes.len() as u64;

        Ok(())
    }

    pub fn write_tile_from_slice(
        &mut self,
        zoom: u8,
        tile_x: i32,
        tile_y: i32,
        cells: &[u8],
    ) -> Result<()> {
        self.write_tile(zoom, tile_x, tile_y, cells.to_vec())
    }

    pub fn tile_count(&self) -> usize {
        self.tile_entries.len()
    }

    pub fn finish(mut self, output_path: &Path, mut metadata: WorldMetadata) -> Result<()> {
        metadata.tile_count = self.tile_entries.len() as u32;
        metadata.tiles = self.tile_entries;

        let probe_bytes = to_bytes::<RkyvError>(&metadata)
            .map_err(|err| anyhow!("failed to serialize world metadata (probe): {err}"))?;
        let metadata_len = probe_bytes.len() as u64;

        let data_offset_base = (WORLD_HEADER_SIZE as u64) + metadata_len;
        for entry in &mut metadata.tiles {
            entry.byte_offset += data_offset_base;
        }

        let metadata_bytes = to_bytes::<RkyvError>(&metadata)
            .map_err(|err| anyhow!("failed to serialize world metadata: {err}"))?;

        ensure!(
            metadata_bytes.len() as u64 == metadata_len,
            "serialized metadata size changed after offset patch"
        );

        self.temp_tile_data_file
            .flush()
            .context("failed to flush temporary world tile bytes")?;
        self.temp_tile_data_file
            .seek(SeekFrom::Start(0))
            .context("failed to rewind temporary world tile bytes")?;

        let mut world_file = File::create(output_path).with_context(|| {
            format!("failed to create world output at {}", output_path.display())
        })?;

        world_file.write_all(WORLD_MAGIC)?;
        world_file.write_all(&WORLD_VERSION.to_le_bytes())?;
        world_file.write_all(&(metadata_len).to_le_bytes())?;
        world_file.write_all(&metadata_bytes)?;

        let copy_progress = ProgressBar::new(self.temp_tile_data_len);
        if let Ok(style) = ProgressStyle::with_template(
            "[{elapsed_precise}] {bar:40.green/blue} {bytes}/{total_bytes} world data",
        ) {
            copy_progress.set_style(style.progress_chars("##-"));
        }

        let mut copy_buffer = vec![0_u8; 1024 * 1024];
        loop {
            let bytes_read = self
                .temp_tile_data_file
                .read(&mut copy_buffer)
                .context("failed while reading temporary world tile data")?;
            if bytes_read == 0 {
                break;
            }

            world_file
                .write_all(&copy_buffer[..bytes_read])
                .context("failed while writing streamed world tile data")?;
            copy_progress.inc(bytes_read as u64);
        }
        copy_progress.finish_with_message("World file write complete");

        drop(self.temp_tile_data_file);
        let _ = std::fs::remove_file(&self.temp_tile_data_path);

        Ok(())
    }
}

impl WorldStreamReader {
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path)
            .with_context(|| format!("failed to open world file {}", path.display()))?;
        let metadata_len = read_header(&mut file)?;

        let mut metadata_bytes = vec![0_u8; metadata_len as usize];
        file.read_exact(&mut metadata_bytes)
            .context("failed to read world metadata bytes")?;

        let archived = access::<ArchivedWorldMetadata, RkyvError>(&metadata_bytes)
            .map_err(|err| anyhow!("failed to access archived world metadata: {err}"))?;

        let mut tiles = HashMap::with_capacity(archived.tiles.len());
        for entry in archived.tiles.iter() {
            tiles.insert(
                (entry.zoom.into(), entry.tile_x.into(), entry.tile_y.into()),
                ChunkLocation {
                    byte_offset: entry.byte_offset.into(),
                    byte_len: entry.byte_len.into(),
                },
            );
        }

        let index = WorldIndex {
            region: archived.region.as_str().to_string(),
            chunk_size_m: archived.chunk_size_m.into(),
            cells_per_side: archived.cells_per_side.into(),
            playable_zoom_level: archived.playable_zoom_level.into(),
            playable_tile_offset_x: archived.playable_tile_offset_x.into(),
            playable_tile_offset_y: archived.playable_tile_offset_y.into(),
            local_bounds_min_x: archived.local_bounds_min_x.into(),
            local_bounds_min_y: archived.local_bounds_min_y.into(),
            local_bounds_max_x: archived.local_bounds_max_x.into(),
            local_bounds_max_y: archived.local_bounds_max_y.into(),
            tiles,
        };

        Ok(Self { index, file })
    }

    pub fn load_tile_bytes(
        &mut self,
        zoom: u8,
        tile_x: i32,
        tile_y: i32,
    ) -> Result<Option<Vec<u8>>> {
        let Some(location) = self.index.tiles.get(&(zoom, tile_x, tile_y)).copied() else {
            return Ok(None);
        };

        let mut data = vec![0_u8; location.byte_len as usize];
        self.file
            .seek(SeekFrom::Start(location.byte_offset))
            .with_context(|| {
                format!(
                    "failed to seek to tile ({zoom}, {tile_x}, {tile_y}) at {}",
                    location.byte_offset
                )
            })?;
        self.file
            .read_exact(&mut data)
            .with_context(|| format!("failed to read tile ({zoom}, {tile_x}, {tile_y}) bytes"))?;

        Ok(Some(data))
    }
}

fn read_header(file: &mut File) -> Result<u64> {
    let mut magic = [0_u8; 8];
    file.read_exact(&mut magic)
        .context("failed to read world magic")?;
    ensure!(magic == *WORLD_MAGIC, "invalid world file magic");

    let mut version_buf = [0_u8; 4];
    file.read_exact(&mut version_buf)
        .context("failed to read world version")?;
    let version = u32::from_le_bytes(version_buf);
    ensure!(
        version == WORLD_VERSION,
        "unsupported world version: {version}"
    );

    let mut metadata_len_buf = [0_u8; 8];
    file.read_exact(&mut metadata_len_buf)
        .context("failed to read world metadata length")?;

    Ok(u64::from_le_bytes(metadata_len_buf))
}

fn build_temp_tile_data_path(output_path: &Path) -> PathBuf {
    let stem = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("world");

    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0_u128, |duration| duration.as_nanos());
    let unique_counter = WORLD_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);

    output_path.with_file_name(format!(
        ".{stem}.{}.{}.{}.tiles.tmp",
        std::process::id(),
        unique_suffix,
        unique_counter
    ))
}
