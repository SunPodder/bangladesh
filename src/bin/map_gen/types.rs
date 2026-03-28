use crate::geometry::Bounds;
use bangladesh::shared::world::{RoadClass, TerrainKind};

#[derive(Debug, Clone)]
pub struct RawAreaFeature {
    pub kind: TerrainKind,
    pub points: Vec<[f64; 2]>,
}

#[derive(Debug, Clone)]
pub struct RawBuildingFeature {
    pub points: Vec<[f64; 2]>,
}

#[derive(Debug, Clone)]
pub struct RawRoadFeature {
    pub class: RoadClass,
    pub width_m: f32,
    pub points: Vec<[f64; 2]>,
}

#[derive(Debug, Clone)]
pub struct RawPoiFeature {
    pub kind: String,
    pub name: Option<String>,
    pub point: [f64; 2],
}

#[derive(Debug, Clone)]
pub struct ParsedMapData {
    pub areas: Vec<RawAreaFeature>,
    pub buildings: Vec<RawBuildingFeature>,
    pub roads: Vec<RawRoadFeature>,
    pub pois: Vec<RawPoiFeature>,
}

#[derive(Debug, Clone, Copy)]
pub struct TileSpec {
    pub id: u32,
    pub grid_x: u32,
    pub grid_y: u32,
    pub bounds: Bounds,
}

#[derive(Debug, Clone)]
pub struct LodSettings {
    pub viewing_distance_m: f32,
    pub simplify_tolerance_m: f32,
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub mercator_bounds: Bounds,
    pub lat_lon_bounds: crate::geometry::LatLonBounds,
    pub total_chunks_estimate: u64,
    pub tile_specs: Vec<TileSpec>,
    pub selected_tile_ids: Vec<u32>,
    pub lods: Vec<LodSettings>,
}
