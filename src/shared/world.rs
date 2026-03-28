use anyhow::{Context, Result, anyhow};
use memmap2::{Mmap, MmapOptions};
use rkyv::{Archive, Deserialize, Serialize, access, rancor::Error as RkyvError, to_bytes};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

pub const MAP_ASSETS_DIR: &str = "assets/map";
pub const TILE_FORMAT_VERSION: u32 = 1;
pub const QUANTIZATION_SCALE: f32 = 100.0;

#[repr(u8)]
#[derive(
    Archive,
    Serialize,
    Deserialize,
    SerdeSerialize,
    SerdeDeserialize,
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
    Road = 7,
}

impl TerrainKind {
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

#[repr(u8)]
#[derive(
    Archive,
    Serialize,
    Deserialize,
    SerdeSerialize,
    SerdeDeserialize,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
)]
pub enum RoadClass {
    Motorway = 0,
    Primary = 1,
    Secondary = 2,
    Local = 3,
    Service = 4,
    Track = 5,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy)]
pub struct QuantizedPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy)]
pub struct QuantizedBounds {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
pub struct AreaFeature {
    pub kind: TerrainKind,
    pub bounds: QuantizedBounds,
    pub rings: Vec<Vec<QuantizedPoint>>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
pub struct BuildingFeature {
    pub bounds: QuantizedBounds,
    pub footprint: Vec<QuantizedPoint>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
pub struct RoadFeature {
    pub class: RoadClass,
    pub width_m: f32,
    pub bounds: QuantizedBounds,
    pub points: Vec<QuantizedPoint>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
pub struct PoiFeature {
    pub kind: String,
    pub name: Option<String>,
    pub point: QuantizedPoint,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
pub struct LodData {
    pub lod_level: u8,
    pub roads: Vec<RoadFeature>,
    pub buildings: Vec<BuildingFeature>,
    pub areas: Vec<AreaFeature>,
    pub pois: Vec<PoiFeature>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
pub struct TileData {
    pub version: u32,
    pub tile_id: u32,
    pub grid_x: u32,
    pub grid_y: u32,
    pub tile_size_m: u32,
    pub origin_x_m: f64,
    pub origin_y_m: f64,
    pub lods: Vec<LodData>,
}

#[derive(SerdeSerialize, SerdeDeserialize, Debug, Clone)]
pub struct MapIndex {
    pub version: u32,
    pub region: String,
    pub source_pbf: String,
    pub lod_count: u8,
    pub lod_viewing_distances_m: Vec<f32>,
    pub lod_simplification_tolerances_m: Vec<f32>,
    pub quantization_scale: f32,
    pub tile_grid: TileGridMetadata,
    pub world_bounds_mercator: BoundsMetadata,
    pub world_bounds_lat_lon: LatLonBoundsMetadata,
    pub tiles: Vec<TileManifest>,
}

#[derive(SerdeSerialize, SerdeDeserialize, Debug, Clone)]
pub struct TileGridMetadata {
    pub origin_x_m: f64,
    pub origin_y_m: f64,
    pub tile_size_m: u32,
    pub cols: u32,
    pub rows: u32,
}

#[derive(SerdeSerialize, SerdeDeserialize, Debug, Clone)]
pub struct BoundsMetadata {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

#[derive(SerdeSerialize, SerdeDeserialize, Debug, Clone)]
pub struct LatLonBoundsMetadata {
    pub min_lat: f64,
    pub min_lon: f64,
    pub max_lat: f64,
    pub max_lon: f64,
}

#[derive(SerdeSerialize, SerdeDeserialize, Debug, Clone, Default)]
pub struct EntityCounts {
    pub roads: usize,
    pub buildings: usize,
    pub areas: usize,
    pub pois: usize,
}

#[derive(SerdeSerialize, SerdeDeserialize, Debug, Clone)]
pub struct TileManifest {
    pub id: u32,
    pub grid_x: u32,
    pub grid_y: u32,
    pub file: String,
    pub file_size_bytes: u64,
    pub origin_x_m: f64,
    pub origin_y_m: f64,
    pub entity_counts: EntityCounts,
}

pub fn map_assets_path() -> PathBuf {
    Path::new(MAP_ASSETS_DIR).to_path_buf()
}

pub fn region_map_path(region: &str) -> PathBuf {
    map_assets_path().join(region)
}

pub fn map_index_path(region: &str) -> PathBuf {
    region_map_path(region).join("map_index.json")
}

pub fn map_tile_path(region: &str, tile_id: u32) -> PathBuf {
    region_map_path(region).join(tile_file_name(tile_id))
}

pub fn tile_file_name(tile_id: u32) -> String {
    format!("tile_{tile_id}.rkyv")
}

pub fn write_tile_file(path: &Path, tile: &TileData) -> Result<()> {
    let bytes = to_bytes::<RkyvError>(tile)
        .map_err(|err| anyhow!("failed to serialize tile {}: {err}", tile.tile_id))?;
    fs::write(path, &bytes).with_context(|| format!("failed to write tile file {}", path.display()))
}

pub fn write_map_index(path: &Path, index: &MapIndex) -> Result<()> {
    let json = serde_json::to_vec_pretty(index).context("failed to serialize map index json")?;
    fs::write(path, json).with_context(|| format!("failed to write map index {}", path.display()))
}

pub fn read_map_index(path: &Path) -> Result<MapIndex> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to read map index {}", path.display()))?;
    serde_json::from_slice(&bytes).context("failed to deserialize map index json")
}

pub struct MappedTile {
    _file: File,
    mmap: Mmap,
}

impl MappedTile {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)
            .with_context(|| format!("failed to open tile file {}", path.display()))?;
        let mmap = unsafe {
            MmapOptions::new()
                .map(&file)
                .with_context(|| format!("failed to mmap tile file {}", path.display()))?
        };

        Ok(Self { _file: file, mmap })
    }

    pub fn archived(&self) -> Result<&ArchivedTileData> {
        access::<ArchivedTileData, RkyvError>(&self.mmap)
            .map_err(|err| anyhow!("failed to access archived tile data: {err}"))
    }
}
