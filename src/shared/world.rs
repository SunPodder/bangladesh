use anyhow::{Context, Result, anyhow, ensure};
use rkyv::{Archive, Deserialize, Serialize, access, rancor::Error as RkyvError, to_bytes};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const WORLD_MAGIC: &[u8; 8] = b"BDWORLD1";
pub const WORLD_VERSION: u32 = 2;
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

pub fn map_assets_path() -> PathBuf {
    Path::new(MAP_ASSETS_DIR).to_path_buf()
}

pub fn world_output_path(region: &str) -> PathBuf {
    map_assets_path().join(format!("{region}.world"))
}

pub fn write_world_file(
    path: &Path,
    mut metadata: WorldMetadata,
    tiles: Vec<TerrainTile>,
) -> Result<()> {
    let mut serialized_tiles: Vec<(u8, i32, i32, rkyv::util::AlignedVec)> =
        Vec::with_capacity(tiles.len());

    for tile in tiles {
        let bytes = to_bytes::<RkyvError>(&tile)
            .map_err(|err| anyhow!("failed to serialize terrain tile: {err}"))?;
        serialized_tiles.push((tile.zoom, tile.tile_x, tile.tile_y, bytes));
    }

    serialized_tiles.sort_by_key(|(zoom, x, y, _)| (*zoom, *y, *x));

    metadata.tiles = serialized_tiles
        .iter()
        .map(|(zoom, tile_x, tile_y, bytes)| TileMetadata {
            zoom: *zoom,
            tile_x: *tile_x,
            tile_y: *tile_y,
            byte_offset: 0,
            byte_len: bytes.len() as u32,
        })
        .collect();
    metadata.tile_count = metadata.tiles.len() as u32;

    let probe_bytes = to_bytes::<RkyvError>(&metadata)
        .map_err(|err| anyhow!("failed to serialize world metadata (probe): {err}"))?;
    let metadata_len = probe_bytes.len() as u64;

    let mut current_offset = (WORLD_HEADER_SIZE as u64) + metadata_len;
    for entry in &mut metadata.tiles {
        entry.byte_offset = current_offset;
        current_offset += u64::from(entry.byte_len);
    }

    let metadata_bytes = to_bytes::<RkyvError>(&metadata)
        .map_err(|err| anyhow!("failed to serialize world metadata: {err}"))?;

    ensure!(
        metadata_bytes.len() as u64 == metadata_len,
        "serialized metadata size changed after offset patch"
    );

    let mut world_file = File::create(path)
        .with_context(|| format!("failed to create world output at {}", path.display()))?;

    world_file.write_all(WORLD_MAGIC)?;
    world_file.write_all(&WORLD_VERSION.to_le_bytes())?;
    world_file.write_all(&(metadata_len).to_le_bytes())?;
    world_file.write_all(&metadata_bytes)?;

    for (_, _, _, bytes) in serialized_tiles {
        world_file.write_all(&bytes)?;
    }

    Ok(())
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
