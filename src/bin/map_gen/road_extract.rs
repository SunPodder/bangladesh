use crate::terrain_types::{RawRoadWay, RoadPolyline};
use anyhow::{Context, Result};
use osmpbf::{Element, ElementReader};
use std::collections::{HashMap, HashSet};
use std::path::Path;

fn classify_road_width_m(way: &osmpbf::Way<'_>) -> Option<f64> {
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

    let highway = highway?;

    let width_m = match highway {
        "motorway" | "trunk" => 14.0,
        "motorway_link" | "trunk_link" => 12.0,
        "primary" => 11.0,
        "primary_link" => 9.0,
        "secondary" => 9.0,
        "secondary_link" => 8.0,
        "tertiary" => 8.0,
        "tertiary_link" => 7.0,
        "residential" | "unclassified" | "living_street" | "road" | "busway" => 7.0,
        "service" => 6.0,
        "track" => 5.0,
        _ => return None,
    };

    Some(width_m)
}

pub fn collect_road_ways(pbf_path: &Path) -> Result<(Vec<RawRoadWay>, HashSet<i64>)> {
    let mut roads = Vec::new();
    let mut needed_nodes = HashSet::new();

    ElementReader::from_path(pbf_path)
        .with_context(|| format!("failed to open pbf file {}", pbf_path.display()))?
        .for_each(|element| {
            let Element::Way(way) = element else {
                return;
            };

            let Some(width_m) = classify_road_width_m(&way) else {
                return;
            };

            let refs: Vec<i64> = way.refs().collect();
            if refs.len() < 2 {
                return;
            }

            needed_nodes.extend(refs.iter().copied());
            roads.push(RawRoadWay {
                width_m,
                node_refs: refs,
            });
        })
        .context("failed during road-way scan")?;

    Ok((roads, needed_nodes))
}

pub fn build_road_polylines(
    ways: Vec<RawRoadWay>,
    node_lookup: &HashMap<i64, [f64; 2]>,
) -> (Vec<RoadPolyline>, usize) {
    let mut polylines = Vec::with_capacity(ways.len());
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

        if missing_node || points.len() < 2 {
            skipped += 1;
            continue;
        }

        polylines.push(RoadPolyline {
            width_m: way.width_m,
            points,
        });
    }

    (polylines, skipped)
}
