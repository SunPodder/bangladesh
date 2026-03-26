use anyhow::{Context, Result, anyhow, ensure};
use rkyv::{
    Archive, Deserialize, Serialize, access, rancor::Error as RkyvError,
    to_bytes,
};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const WORLD_MAGIC: &[u8; 8] = b"BDWORLD1";
pub const WORLD_VERSION: u32 = 1;
const WORLD_HEADER_SIZE: usize = 8 + 4 + 8;

#[repr(u8)]
#[derive(
    Archive,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
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
pub struct TerrainChunk {
    pub chunk_x: i32,
    pub chunk_y: i32,
    pub cells: Vec<u8>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
pub struct ChunkMetadata {
    pub chunk_x: i32,
    pub chunk_y: i32,
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
    pub mercator_origin_x_m: f64,
    pub mercator_origin_y_m: f64,
    pub local_bounds_min_x: f32,
    pub local_bounds_min_y: f32,
    pub local_bounds_max_x: f32,
    pub local_bounds_max_y: f32,
    pub chunk_count: u32,
    pub chunks: Vec<ChunkMetadata>,
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
    pub local_bounds_min_x: f32,
    pub local_bounds_min_y: f32,
    pub local_bounds_max_x: f32,
    pub local_bounds_max_y: f32,
    pub chunks: HashMap<(i32, i32), ChunkLocation>,
}

#[derive(Debug)]
pub struct WorldStreamReader {
    pub index: WorldIndex,
    file: File,
}

pub fn world_output_path(region: &str) -> PathBuf {
    Path::new("assets/data/processed").join(format!("{region}.world"))
}

pub fn write_world_file(
    path: &Path,
    mut metadata: WorldMetadata,
    chunks: Vec<TerrainChunk>,
) -> Result<()> {
    let mut serialized_chunks: Vec<(i32, i32, rkyv::util::AlignedVec)> =
        Vec::with_capacity(chunks.len());

    for chunk in chunks {
        let bytes = to_bytes::<RkyvError>(&chunk)
            .map_err(|err| anyhow!("failed to serialize terrain chunk: {err}"))?;
        serialized_chunks.push((chunk.chunk_x, chunk.chunk_y, bytes));
    }

    serialized_chunks.sort_by_key(|(x, y, _)| (*y, *x));

    metadata.chunks = serialized_chunks
        .iter()
        .map(|(chunk_x, chunk_y, bytes)| ChunkMetadata {
            chunk_x: *chunk_x,
            chunk_y: *chunk_y,
            byte_offset: 0,
            byte_len: bytes.len() as u32,
        })
        .collect();
    metadata.chunk_count = metadata.chunks.len() as u32;

    let probe_bytes = to_bytes::<RkyvError>(&metadata)
        .map_err(|err| anyhow!("failed to serialize world metadata (probe): {err}"))?;
    let metadata_len = probe_bytes.len() as u64;

    let mut current_offset = (WORLD_HEADER_SIZE as u64) + metadata_len;
    for entry in &mut metadata.chunks {
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

    for (_, _, bytes) in serialized_chunks {
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

        let mut chunks = HashMap::with_capacity(archived.chunks.len());
        for entry in archived.chunks.iter() {
            chunks.insert(
                (entry.chunk_x.into(), entry.chunk_y.into()),
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
            local_bounds_min_x: archived.local_bounds_min_x.into(),
            local_bounds_min_y: archived.local_bounds_min_y.into(),
            local_bounds_max_x: archived.local_bounds_max_x.into(),
            local_bounds_max_y: archived.local_bounds_max_y.into(),
            chunks,
        };

        Ok(Self { index, file })
    }

    pub fn load_chunk_bytes(
        &mut self,
        chunk_x: i32,
        chunk_y: i32,
    ) -> Result<Option<Vec<u8>>> {
        let Some(location) = self.index.chunks.get(&(chunk_x, chunk_y)).copied() else {
            return Ok(None);
        };

        let mut data = vec![0_u8; location.byte_len as usize];
        self.file
            .seek(SeekFrom::Start(location.byte_offset))
            .with_context(|| {
                format!(
                    "failed to seek to chunk ({chunk_x}, {chunk_y}) at {}",
                    location.byte_offset
                )
            })?;
        self.file
            .read_exact(&mut data)
            .with_context(|| format!("failed to read chunk ({chunk_x}, {chunk_y}) bytes"))?;

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