use bangladesh::shared::world::TerrainKind;

#[allow(dead_code)]
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
        | ("waterway", "riverbank")
        | ("landuse", "reservoir")
        | ("leisure", "swimming_area") => Some(TerrainKind::Water),

        ("landuse", "forest")
        | ("natural", "wood")
        | ("landuse", "wood")
        | ("natural", "heath")
        | ("natural", "scrub")
        | ("natural", "tree_row") => Some(TerrainKind::Forest),

        ("landuse", "residential")
        | ("landuse", "commercial")
        | ("landuse", "industrial")
        | ("landuse", "retail")
        | ("landuse", "construction")
        | ("landuse", "brownfield")
        | ("landuse", "depot")
        | ("landuse", "Dhaka Woasa")
        | ("landuse", "education")
        | ("landuse", "garages")
        | ("landuse", "landfill")
        | ("landuse", "military")
        | ("landuse", "religious")
        | ("landuse", "slam")
        | ("landuse", "slum")
        | ("landuse", "yes")
        | ("building", _)
        | ("amenity", _)
        | ("office", _)
        | ("leisure", "bleachers")
        | ("leisure", "fitness_station")
        | ("leisure", "outdoor_seating")
        | ("leisure", "resort")
        | ("leisure", "slipway")
        | ("leisure", "sports_centre")
        | ("leisure", "stadium")
        | ("leisure", "swimming_pool") => Some(TerrainKind::Urban),

        ("landuse", "farmland")
        | ("landuse", "allotments")
        | ("landuse", "farmyard")
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
        | ("landuse", "cemetery")
        | ("landuse", "churchyard")
        | ("landuse", "common")
        | ("landuse", "flowerbed")
        | ("landuse", "greenfield")
        | ("landuse", "open space")
        | ("landuse", "open_space")
        | ("landuse", "playground")
        | ("leisure", "common")
        | ("leisure", "garden")
        | ("leisure", "golf_course")
        | ("leisure", "open space")
        | ("leisure", "Open space")
        | ("leisure", "park")
        | ("leisure", "pitch")
        | ("leisure", "playground")
        | ("leisure", "practice_pitch")
        | ("leisure", "track")
        | ("natural", "grassland")
        | ("natural", "yes") => Some(TerrainKind::Grass),

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
