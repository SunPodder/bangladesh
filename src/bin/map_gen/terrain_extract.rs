use crate::constants::DEFAULT_TERRAIN;
use crate::geometry::lat_lon_to_web_mercator;
use crate::terrain_types::{RawTerrainWay, TerrainPolygon};
use anyhow::{Context, Result};
use bangladesh::shared::world::TerrainKind;
use osmpbf::{Element, ElementReader};
use std::collections::{HashMap, HashSet};
use std::path::Path;

fn classify_way_terrain(way: &osmpbf::Way<'_>) -> Option<TerrainKind> {
    let mut best_match: Option<TerrainKind> = None;
    let mut has_area_hint = false;

    for (key, value) in way.tags() {
        if matches!(key, "landuse" | "natural" | "leisure") {
            has_area_hint = true;
        }

        let terrain = match (key, value) {
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
            | ("landuse", "construction") => Some(TerrainKind::Urban),

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
        };

        if let Some(candidate) = terrain {
            best_match = match best_match {
                Some(existing) if existing.priority() >= candidate.priority() => Some(existing),
                _ => Some(candidate),
            };
        }
    }

    if best_match.is_some() {
        best_match
    } else if has_area_hint {
        Some(DEFAULT_TERRAIN)
    } else {
        None
    }
}

pub fn collect_terrain_ways(pbf_path: &Path) -> Result<(Vec<RawTerrainWay>, HashSet<i64>)> {
    let mut ways = Vec::new();
    let mut needed_nodes = HashSet::new();

    ElementReader::from_path(pbf_path)
        .with_context(|| format!("failed to open pbf file {}", pbf_path.display()))?
        .for_each(|element| {
            let Element::Way(way) = element else {
                return;
            };

            let Some(terrain) = classify_way_terrain(&way) else {
                return;
            };

            let refs: Vec<i64> = way.refs().collect();
            if refs.len() < 4 || refs.first() != refs.last() {
                return;
            }

            needed_nodes.extend(refs.iter().copied());
            ways.push(RawTerrainWay {
                terrain,
                node_refs: refs,
            });
        })
        .context("failed during terrain-way scan")?;

    Ok((ways, needed_nodes))
}

pub fn collect_needed_nodes(
    pbf_path: &Path,
    needed_nodes: &HashSet<i64>,
) -> Result<HashMap<i64, [f64; 2]>> {
    let mut node_lookup = HashMap::with_capacity(needed_nodes.len());

    ElementReader::from_path(pbf_path)
        .with_context(|| format!("failed to open pbf file {}", pbf_path.display()))?
        .for_each(|element| match element {
            Element::Node(node) => {
                if needed_nodes.contains(&node.id()) {
                    node_lookup
                        .entry(node.id())
                        .or_insert_with(|| lat_lon_to_web_mercator(node.lat(), node.lon()));
                }
            }
            Element::DenseNode(node) => {
                if needed_nodes.contains(&node.id()) {
                    node_lookup
                        .entry(node.id())
                        .or_insert_with(|| lat_lon_to_web_mercator(node.lat(), node.lon()));
                }
            }
            _ => {}
        })
        .context("failed during node scan")?;

    Ok(node_lookup)
}

pub fn build_polygons(
    ways: Vec<RawTerrainWay>,
    node_lookup: &HashMap<i64, [f64; 2]>,
) -> (Vec<TerrainPolygon>, usize) {
    let mut polygons = Vec::with_capacity(ways.len());
    let mut skipped = 0_usize;

    for way in ways {
        let mut points = Vec::with_capacity(way.node_refs.len());
        let mut missing_node = false;

        for node_id in way.node_refs {
            let Some(position) = node_lookup.get(&node_id) else {
                missing_node = true;
                break;
            };
            points.push(*position);
        }

        if missing_node || points.len() < 4 {
            skipped += 1;
            continue;
        }

        polygons.push(TerrainPolygon {
            terrain: way.terrain,
            points,
        });
    }

    (polygons, skipped)
}
