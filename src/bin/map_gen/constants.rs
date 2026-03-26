use bangladesh::shared::world::TerrainKind;

pub const GIS_TO_WORLD_SCALE: f64 = 1.0;
pub const CHUNK_SIZE_METERS: f64 = 1024.0;
pub const DEFAULT_CELLS_PER_SIDE: usize = 256;
pub const WEB_MERCATOR_MAX_LAT: f64 = 85.05112878;
pub const DEFAULT_TERRAIN: TerrainKind = TerrainKind::Grass;
