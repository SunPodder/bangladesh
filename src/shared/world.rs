use anyhow::{Context, Result, anyhow, ensure};
use rkyv::{Archive, Deserialize, Serialize, access, rancor::Error as RkyvError, to_bytes};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const WORLD_MAGIC: &[u8; 8] = b"BDWORLD1";
pub const WORLD_VERSION_V2: u32 = 2;
pub const WORLD_VERSION_V3: u32 = 3;
pub const WORLD_VERSION: u32 = WORLD_VERSION_V3;
const WORLD_HEADER_SIZE: usize = 8 + 4 + 8;
const MAP_ASSETS_DIR: &str = "assets/map";

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
    Road = 7,
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
            7 => Self::Road,
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
            Self::Road => 7,
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
    output_path: PathBuf,
    world_file: File,
    tile_entries: Vec<TileMetadata>,
    tile_data_len: u64,
}

enum HeaderInfo {
    V2 { metadata_len: u64 },
    V3 { metadata_offset: u64 },
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
        let mut world_file = File::create(output_path).with_context(|| {
            format!("failed to create world output at {}", output_path.display())
        })?;
        world_file
            .write_all(WORLD_MAGIC)
            .context("failed to write world magic")?;
        world_file
            .write_all(&WORLD_VERSION.to_le_bytes())
            .context("failed to write world version")?;
        world_file
            .write_all(&0_u64.to_le_bytes())
            .context("failed to write world metadata offset placeholder")?;

        Ok(Self {
            output_path: output_path.to_path_buf(),
            world_file,
            tile_entries: Vec::new(),
            tile_data_len: 0,
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

        let tile_offset = (WORLD_HEADER_SIZE as u64) + self.tile_data_len;

        self.world_file
            .write_all(&bytes)
            .with_context(|| {
                format!(
                    "failed to append serialized tile bytes to {}",
                    self.output_path.display()
                )
            })?;

        self.tile_entries.push(TileMetadata {
            zoom,
            tile_x,
            tile_y,
            byte_offset: tile_offset,
            byte_len: bytes.len() as u32,
        });
        self.tile_data_len += bytes.len() as u64;

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

        let metadata_bytes = to_bytes::<RkyvError>(&metadata)
            .map_err(|err| anyhow!("failed to serialize world metadata: {err}"))?;

        let metadata_offset = (WORLD_HEADER_SIZE as u64) + self.tile_data_len;
        self.world_file
            .write_all(&metadata_bytes)
            .context("failed to write world metadata trailer")?;

        self.world_file
            .seek(SeekFrom::Start((WORLD_MAGIC.len() + std::mem::size_of::<u32>()) as u64))
            .context("failed to seek to metadata pointer slot")?;
        self.world_file
            .write_all(&metadata_offset.to_le_bytes())
            .context("failed to backfill metadata pointer")?;
        self.world_file
            .flush()
            .context("failed to flush world output file")?;

        let _ = output_path;

        Ok(())
    }
}

impl WorldStreamReader {
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path)
            .with_context(|| format!("failed to open world file {}", path.display()))?;
        let header = read_header(&mut file)?;

        let metadata_bytes = match header {
            HeaderInfo::V2 { metadata_len } => {
                let mut bytes = vec![0_u8; metadata_len as usize];
                file.read_exact(&mut bytes)
                    .context("failed to read v2 world metadata bytes")?;
                bytes
            }
            HeaderInfo::V3 { metadata_offset } => {
                let file_len = file
                    .metadata()
                    .with_context(|| format!("failed to stat world file {}", path.display()))?
                    .len();
                ensure!(
                    metadata_offset <= file_len,
                    "invalid metadata offset in world file"
                );
                let metadata_len = file_len - metadata_offset;
                ensure!(
                    usize::try_from(metadata_len).is_ok(),
                    "world metadata too large for this platform"
                );

                file.seek(SeekFrom::Start(metadata_offset))
                    .context("failed to seek to v3 world metadata")?;
                let mut bytes = vec![0_u8; metadata_len as usize];
                file.read_exact(&mut bytes)
                    .context("failed to read v3 world metadata bytes")?;
                bytes
            }
        };

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

fn read_header(file: &mut File) -> Result<HeaderInfo> {
    let mut magic = [0_u8; 8];
    file.read_exact(&mut magic)
        .context("failed to read world magic")?;
    ensure!(magic == *WORLD_MAGIC, "invalid world file magic");

    let mut version_buf = [0_u8; 4];
    file.read_exact(&mut version_buf)
        .context("failed to read world version")?;
    let version = u32::from_le_bytes(version_buf);

    let mut metadata_ptr_buf = [0_u8; 8];
    file.read_exact(&mut metadata_ptr_buf)
        .context("failed to read world metadata pointer")?;
    let metadata_ptr = u64::from_le_bytes(metadata_ptr_buf);

    match version {
        WORLD_VERSION_V2 => Ok(HeaderInfo::V2 {
            metadata_len: metadata_ptr,
        }),
        WORLD_VERSION_V3 => Ok(HeaderInfo::V3 {
            metadata_offset: metadata_ptr,
        }),
        _ => Err(anyhow!("unsupported world version: {version}")),
    }
}
