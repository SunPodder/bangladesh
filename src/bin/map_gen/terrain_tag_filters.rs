use bangladesh::shared::world::TerrainKind;

pub fn is_area_hint_key(key: &str) -> bool {
    matches!(
        key,
        "landuse" | "natural" | "leisure" | "building" | "amenity" | "office"
    )
}

pub fn classify_tag_pair(key: &str, value: &str) -> Option<TerrainKind> {
    match (key, value) {
        ("natural", "water")
        | ("natural", "wetland")
        | ("natural", "bay")
        | ("natural", "coastline")
        | ("waterway", "riverbank") => Some(TerrainKind::Water),

        ("landuse", "forest") | ("natural", "wood") | ("landuse", "wood") => {
            Some(TerrainKind::Forest)
        }

        ("landuse", "residential")
        | ("landuse", "commercial")
        | ("landuse", "industrial")
        | ("landuse", "retail")
        | ("landuse", "construction")
        | ("building", _)
        | ("amenity", _)
        | ("office", _) => Some(TerrainKind::Urban),

        ("landuse", "farmland")
        | ("landuse", "orchard")
        | ("landuse", "vineyard")
        | ("landuse", "greenhouse_horticulture")
        | ("landuse", "plant_nursery")
        | ("landuse", "plantation") => Some(TerrainKind::Farmland),

        ("natural", "sand") | ("natural", "beach") => Some(TerrainKind::Sand),

        ("landuse", "grass")
        | ("landuse", "meadow")
        | ("landuse", "village_green")
        | ("landuse", "recreation_ground")
        | ("natural", "grassland") => Some(TerrainKind::Grass),

        _ => None,
    }
}

pub fn best_terrain_match<'a, I>(tags: I) -> Option<TerrainKind>
where
    I: Iterator<Item = (&'a str, &'a str)>,
{
    let mut best_match: Option<TerrainKind> = None;

    for (key, value) in tags {
        let Some(candidate) = classify_tag_pair(key, value) else {
            continue;
        };

        best_match = match best_match {
            Some(existing) if existing.priority() >= candidate.priority() => Some(existing),
            _ => Some(candidate),
        };
    }

    best_match
}
