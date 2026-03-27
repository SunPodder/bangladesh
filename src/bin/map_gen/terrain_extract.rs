use crate::constants::DEFAULT_TERRAIN;
use crate::geometry::lat_lon_to_web_mercator;
use crate::terrain_tag_filters::{best_terrain_match, classify_tag_pair, is_area_hint_key};
use crate::terrain_types::{RawTerrainWay, TerrainPolygon};
use anyhow::{Context, Result};
use bangladesh::shared::world::TerrainKind;
use osmpbf::{Element, ElementReader, RelMemberType};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Clone)]
struct TerrainRelation {
    terrain: TerrainKind,
    outer_way_ids: Vec<i64>,
}

fn classify_terrain_tags<'a, I>(tags: I) -> Option<TerrainKind>
where
    I: Iterator<Item = (&'a str, &'a str)>,
{
    let tag_pairs: Vec<(&str, &str)> = tags.collect();
    if let Some(best_match) = best_terrain_match(tag_pairs.iter().copied()) {
        return Some(best_match);
    }

    let unmatched_area_tags: Vec<String> = tag_pairs
        .iter()
        .copied()
        .filter(|(key, value)| is_area_hint_key(key) && classify_tag_pair(key, value).is_none())
        .map(|(key, value)| format!("{key}={value}"))
        .collect();

    if !unmatched_area_tags.is_empty() {
        eprintln!(
            "terrain_extract: default terrain fallback for unmatched tags: {}",
            unmatched_area_tags.join(", ")
        );
        Some(DEFAULT_TERRAIN)
    } else {
        Some(DEFAULT_TERRAIN)
    }
}

fn classify_way_terrain(way: &osmpbf::Way<'_>) -> Option<TerrainKind> {
    classify_terrain_tags(way.tags())
}

fn classify_relation_terrain(relation: &osmpbf::Relation<'_>) -> Option<TerrainKind> {
    let mut is_multipolygon = false;
    let mut tag_pairs = Vec::new();

    for (key, value) in relation.tags() {
        if key == "type" && value == "multipolygon" {
            is_multipolygon = true;
        }
        tag_pairs.push((key, value));
    }

    if !is_multipolygon {
        return None;
    }

    classify_terrain_tags(tag_pairs.into_iter())
}

fn stitch_relation_outer_rings(
    outer_way_ids: &[i64],
    relation_way_refs: &HashMap<i64, Vec<i64>>,
) -> Vec<Vec<i64>> {
    let mut segments: Vec<Vec<i64>> = Vec::new();

    for way_id in outer_way_ids {
        let Some(node_refs) = relation_way_refs.get(way_id) else {
            continue;
        };

        if node_refs.len() < 2 {
            continue;
        }

        segments.push(node_refs.clone());
    }

    let mut rings = Vec::new();

    while !segments.is_empty() {
        let mut ring = segments.swap_remove(segments.len() - 1);
        if ring.len() < 2 {
            continue;
        }

        while ring.first() != ring.last() {
            let tail = *ring.last().unwrap_or(&0);
            let mut next_match: Option<(usize, bool)> = None;

            for (index, segment) in segments.iter().enumerate() {
                if segment.is_empty() {
                    continue;
                }

                if segment[0] == tail {
                    next_match = Some((index, false));
                    break;
                }

                if *segment.last().unwrap_or(&0) == tail {
                    next_match = Some((index, true));
                    break;
                }
            }

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

pub fn collect_terrain_ways(pbf_path: &Path) -> Result<(Vec<RawTerrainWay>, HashSet<i64>)> {
    let mut closed_ways = Vec::new();
    let mut terrain_relations = Vec::new();
    let mut relation_way_ids = HashSet::new();

    ElementReader::from_path(pbf_path)
        .with_context(|| format!("failed to open pbf file {}", pbf_path.display()))?
        .for_each(|element| match element {
            Element::Way(way) => {
                let Some(terrain) = classify_way_terrain(&way) else {
                    return;
                };

                let refs: Vec<i64> = way.refs().collect();
                if refs.len() < 4 || refs.first() != refs.last() {
                    return;
                }

                closed_ways.push(RawTerrainWay {
                    terrain,
                    node_refs: refs,
                });
            }
            Element::Relation(relation) => {
                let Some(terrain) = classify_relation_terrain(&relation) else {
                    return;
                };

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
                    terrain_relations.push(TerrainRelation {
                        terrain,
                        outer_way_ids,
                    });
                }
            }
            _ => {}
        })
        .context("failed during terrain-way/relation scan")?;

    let mut relation_way_refs = HashMap::with_capacity(relation_way_ids.len());
    if !relation_way_ids.is_empty() {
        ElementReader::from_path(pbf_path)
            .with_context(|| format!("failed to open pbf file {}", pbf_path.display()))?
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
            .context("failed during relation-way member scan")?;
    }

    let mut ways = closed_ways;
    for relation in terrain_relations {
        let rings = stitch_relation_outer_rings(&relation.outer_way_ids, &relation_way_refs);
        for ring in rings {
            ways.push(RawTerrainWay {
                terrain: relation.terrain,
                node_refs: ring,
            });
        }
    }

    let mut needed_nodes = HashSet::new();
    for way in &ways {
        needed_nodes.extend(way.node_refs.iter().copied());
    }

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
