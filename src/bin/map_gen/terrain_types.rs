use bangladesh::shared::world::TerrainKind;

#[derive(Clone)]
pub struct RawTerrainWay {
    pub terrain: TerrainKind,
    pub node_refs: Vec<i64>,
}

#[derive(Clone)]
pub struct TerrainPolygon {
    pub terrain: TerrainKind,
    pub points: Vec<[f64; 2]>,
}

#[derive(Clone)]
pub struct RawRoadWay {
    pub width_m: f64,
    pub node_refs: Vec<i64>,
}

#[derive(Clone)]
pub struct RoadPolyline {
    pub width_m: f64,
    pub points: Vec<[f64; 2]>,
}
