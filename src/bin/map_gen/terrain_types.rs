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
