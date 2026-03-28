use crate::geometry::{Bounds, lat_lon_to_web_mercator};
use crate::terrain_tag_filters::best_terrain_match;
use crate::types::{
    ParsedMapData, RawAreaFeature, RawBuildingFeature, RawPoiFeature, RawRoadFeature,
};
use anyhow::{Context, Result};
use bangladesh::shared::world::{RoadClass, TerrainKind};
use osmpbf::{Element, ElementReader, RelMemberType};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Clone)]
struct RawWayRecord {
    kind: WayKind,
    node_refs: Vec<i64>,
}

#[derive(Clone)]
struct RelationRecord {
    terrain: TerrainKind,
    outer_way_ids: Vec<i64>,
}

#[derive(Clone)]
enum WayKind {
    Area(TerrainKind),
    Building,
    Road { class: RoadClass, width_m: f32 },
}

pub fn parse_map_data(pbf_path: &Path) -> Result<ParsedMapData> {
    let mut way_records = Vec::new();
    let mut relation_records = Vec::new();
    let mut relation_way_ids = HashSet::new();
    let mut pois = Vec::new();

    ElementReader::from_path(pbf_path)
        .with_context(|| format!("failed to open pbf file {}", pbf_path.display()))?
        .for_each(|element| match element {
            Element::Way(way) => {
                let refs: Vec<i64> = way.refs().collect();
                if let Some(kind) = classify_way(&way) {
                    match kind {
                        WayKind::Area(_) | WayKind::Building
                            if refs.len() >= 4 && refs.first() == refs.last() =>
                        {
                            way_records.push(RawWayRecord {
                                kind,
                                node_refs: refs,
                            });
                        }
                        WayKind::Road { .. } if refs.len() >= 2 => {
                            way_records.push(RawWayRecord {
                                kind,
                                node_refs: refs,
                            });
                        }
                        _ => {}
                    }
                }
            }
            Element::Relation(relation) => {
                if let Some(terrain) = classify_relation_area(&relation) {
                    let mut outer_way_ids = Vec::new();
                    for member in relation.members() {
                        if member.member_type != RelMemberType::Way {
                            continue;
                        }
                        let role = member.role().unwrap_or("");
                        if role == "outer" || role.is_empty() {
                            outer_way_ids.push(member.member_id);
                            relation_way_ids.insert(member.member_id);
                        }
                    }
                    if !outer_way_ids.is_empty() {
                        relation_records.push(RelationRecord {
                            terrain,
                            outer_way_ids,
                        });
                    }
                }
            }
            Element::Node(node) => {
                if let Some(poi) = classify_poi(node.tags(), node.lat(), node.lon()) {
                    pois.push(poi);
                }
            }
            Element::DenseNode(node) => {
                if let Some(poi) = classify_poi(node.tags(), node.lat(), node.lon()) {
                    pois.push(poi);
                }
            }
        })
        .context("failed while scanning ways, relations, and POIs")?;

    let mut relation_way_refs = HashMap::with_capacity(relation_way_ids.len());
    if !relation_way_ids.is_empty() {
        ElementReader::from_path(pbf_path)
            .with_context(|| format!("failed to reopen pbf file {}", pbf_path.display()))?
            .for_each(|element| {
                let Element::Way(way) = element else {
                    return;
                };
                if !relation_way_ids.contains(&way.id()) {
                    return;
                }
                let refs: Vec<i64> = way.refs().collect();
                if refs.len() >= 2 {
                    relation_way_refs.insert(way.id(), refs);
                }
            })
            .context("failed while collecting relation way members")?;
    }

    let mut needed_nodes = HashSet::new();
    for record in &way_records {
        needed_nodes.extend(record.node_refs.iter().copied());
    }
    for relation in &relation_records {
        for rings in stitch_relation_outer_rings(&relation.outer_way_ids, &relation_way_refs) {
            needed_nodes.extend(rings);
        }
    }

    let node_lookup = collect_needed_nodes(pbf_path, &needed_nodes)?;

    let mut areas = Vec::new();
    let mut buildings = Vec::new();
    let mut roads = Vec::new();

    for record in way_records {
        match record.kind {
            WayKind::Area(kind) => {
                if let Some(points) = resolve_points(&record.node_refs, &node_lookup, true) {
                    areas.push(RawAreaFeature { kind, points });
                }
            }
            WayKind::Building => {
                if let Some(points) = resolve_points(&record.node_refs, &node_lookup, true) {
                    buildings.push(RawBuildingFeature { points });
                }
            }
            WayKind::Road { class, width_m } => {
                if let Some(points) = resolve_points(&record.node_refs, &node_lookup, false) {
                    roads.push(RawRoadFeature {
                        class,
                        width_m,
                        points,
                    });
                }
            }
        }
    }

    for relation in relation_records {
        for ring in stitch_relation_outer_rings(&relation.outer_way_ids, &relation_way_refs) {
            if let Some(points) = resolve_points(&ring, &node_lookup, true) {
                areas.push(RawAreaFeature {
                    kind: relation.terrain,
                    points,
                });
            }
        }
    }

    Ok(ParsedMapData {
        areas,
        buildings,
        roads,
        pois,
    })
}

fn classify_way(way: &osmpbf::Way<'_>) -> Option<WayKind> {
    if has_tag(way.tags(), "building") {
        return Some(WayKind::Building);
    }

    if let Some((class, width_m)) = classify_road(way) {
        return Some(WayKind::Road { class, width_m });
    }

    best_terrain_match(way.tags()).map(WayKind::Area)
}

fn classify_relation_area(relation: &osmpbf::Relation<'_>) -> Option<TerrainKind> {
    let mut is_multipolygon = false;
    let mut tags = Vec::new();

    for (key, value) in relation.tags() {
        if key == "type" && value == "multipolygon" {
            is_multipolygon = true;
        }
        tags.push((key, value));
    }

    if !is_multipolygon {
        return None;
    }

    best_terrain_match(tags.into_iter())
}

fn classify_road(way: &osmpbf::Way<'_>) -> Option<(RoadClass, f32)> {
    let mut highway = None;
    let mut area_yes = false;

    for (key, value) in way.tags() {
        if key == "highway" {
            highway = Some(value);
        } else if key == "area" && value == "yes" {
            area_yes = true;
        }
    }

    if area_yes {
        return None;
    }

    match highway? {
        "motorway" | "trunk" | "motorway_link" | "trunk_link" => Some((RoadClass::Motorway, 14.0)),
        "primary" | "primary_link" => Some((RoadClass::Primary, 11.0)),
        "secondary" | "secondary_link" | "tertiary" | "tertiary_link" => {
            Some((RoadClass::Secondary, 8.5))
        }
        "residential" | "unclassified" | "living_street" | "road" | "busway" => {
            Some((RoadClass::Local, 7.0))
        }
        "service" => Some((RoadClass::Service, 6.0)),
        "track" => Some((RoadClass::Track, 5.0)),
        _ => None,
    }
}

fn classify_poi<'a, I>(tags: I, lat: f64, lon: f64) -> Option<RawPoiFeature>
where
    I: Iterator<Item = (&'a str, &'a str)>,
{
    let mut kind = None;
    let mut name = None;

    for (key, value) in tags {
        if key == "name" {
            name = Some(value.to_string());
        }

        let candidate = match (key, value) {
            ("amenity", value) => Some(format!("amenity:{value}")),
            ("tourism", value) => Some(format!("tourism:{value}")),
            ("railway", "station") => Some("railway:station".to_string()),
            ("aeroway", "aerodrome") => Some("aeroway:aerodrome".to_string()),
            ("place", value @ ("city" | "town" | "village")) => Some(format!("place:{value}")),
            _ => None,
        };

        if candidate.is_some() && kind.is_none() {
            kind = candidate;
        }
    }

    let kind = kind?;
    Some(RawPoiFeature {
        kind,
        name,
        point: lat_lon_to_web_mercator(lat, lon),
    })
}

fn collect_needed_nodes(
    pbf_path: &Path,
    needed_nodes: &HashSet<i64>,
) -> Result<HashMap<i64, [f64; 2]>> {
    let mut node_lookup = HashMap::with_capacity(needed_nodes.len());

    ElementReader::from_path(pbf_path)
        .with_context(|| format!("failed to reopen pbf file {}", pbf_path.display()))?
        .for_each(|element| match element {
            Element::Node(node) => {
                if needed_nodes.contains(&node.id()) {
                    node_lookup.insert(node.id(), lat_lon_to_web_mercator(node.lat(), node.lon()));
                }
            }
            Element::DenseNode(node) => {
                if needed_nodes.contains(&node.id()) {
                    node_lookup.insert(node.id(), lat_lon_to_web_mercator(node.lat(), node.lon()));
                }
            }
            _ => {}
        })
        .context("failed while collecting referenced nodes")?;

    Ok(node_lookup)
}

fn resolve_points(
    node_refs: &[i64],
    node_lookup: &HashMap<i64, [f64; 2]>,
    require_closed: bool,
) -> Option<Vec<[f64; 2]>> {
    let mut points = Vec::with_capacity(node_refs.len());
    for node_id in node_refs {
        points.push(*node_lookup.get(node_id)?);
    }

    if require_closed {
        if points.len() < 4 {
            return None;
        }
        if points.first() != points.last() {
            points.push(*points.first()?);
        }
    } else if points.len() < 2 {
        return None;
    }

    Some(points)
}

fn stitch_relation_outer_rings(
    outer_way_ids: &[i64],
    relation_way_refs: &HashMap<i64, Vec<i64>>,
) -> Vec<Vec<i64>> {
    let mut segments: Vec<Vec<i64>> = outer_way_ids
        .iter()
        .filter_map(|way_id| relation_way_refs.get(way_id).cloned())
        .filter(|segment| segment.len() >= 2)
        .collect();

    let mut rings = Vec::new();
    while let Some(mut ring) = segments.pop() {
        while ring.first() != ring.last() {
            let tail = *ring.last().unwrap_or(&0);
            let next_match = segments.iter().enumerate().find_map(|(index, segment)| {
                if segment.first().copied() == Some(tail) {
                    Some((index, false))
                } else if segment.last().copied() == Some(tail) {
                    Some((index, true))
                } else {
                    None
                }
            });

            let Some((index, reverse)) = next_match else {
                break;
            };

            let segment = segments.swap_remove(index);
            if reverse {
                for node_id in segment.iter().rev().skip(1) {
                    ring.push(*node_id);
                }
            } else {
                ring.extend(segment.into_iter().skip(1));
            }
        }

        if ring.first() == ring.last() && ring.len() >= 4 {
            rings.push(ring);
        }
    }

    rings
}

fn has_tag<'a, I>(mut tags: I, key_to_find: &str) -> bool
where
    I: Iterator<Item = (&'a str, &'a str)>,
{
    tags.any(|(key, _)| key == key_to_find)
}

#[allow(dead_code)]
fn compute_bounds(points: &[[f64; 2]]) -> Bounds {
    let mut bounds = Bounds::new(points[0]);
    for point in points.iter().copied().skip(1) {
        bounds.include(point);
    }
    bounds
}
