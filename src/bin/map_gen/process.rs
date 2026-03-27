use crate::constants::{CHUNK_SIZE_METERS, GIS_TO_WORLD_SCALE};
use crate::geometry::compute_global_bounds_for_features;
use crate::pyramid::{PyramidStreamReducer, compute_pyramid_layout};
use crate::rasterize::{
    build_chunk_polygon_index, build_chunk_road_index, stream_rasterized_chunks,
};
use crate::road_extract::{build_road_polylines, collect_road_ways};
use crate::terrain_extract::{build_polygons, collect_needed_nodes, collect_terrain_ways};
use anyhow::{Context, Result, anyhow, ensure};
use bangladesh::shared::world::{WorldMetadata, WorldWriter, world_output_path};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn process_terrain_world(
    region: &str,
    raw_file_path: &Path,
    cells_per_side: usize,
    raster_memory_gib: f64,
) -> Result<()> {
    ensure!(cells_per_side >= 2, "cells_per_side must be at least 2");
    ensure!(
        cells_per_side % 2 == 0,
        "cells_per_side must be even for 2x downsampling"
    );
    ensure!(
        u16::try_from(cells_per_side).is_ok(),
        "cells_per_side must fit into u16 metadata"
    );
    ensure!(
        raster_memory_gib.is_finite() && raster_memory_gib > 0.0,
        "raster_memory_gib must be a positive finite number"
    );

    let raster_memory_budget_bytes =
        (raster_memory_gib * 1024.0 * 1024.0 * 1024.0).round() as u64;

    println!("Scanning terrain and road ways from {:?}", raw_file_path);
    let (ways, mut needed_nodes) = collect_terrain_ways(raw_file_path)?;
    let (road_ways, road_needed_nodes) = collect_road_ways(raw_file_path)?;
    needed_nodes.extend(road_needed_nodes);

    ensure!(
        !ways.is_empty() || !road_ways.is_empty(),
        "no terrain or road features found in {:?}",
        raw_file_path
    );

    println!(
        "Collected {} terrain ways, {} road ways, and {} required nodes",
        ways.len(),
        road_ways.len(),
        needed_nodes.len()
    );

    let node_lookup = collect_needed_nodes(raw_file_path, &needed_nodes)?;
    println!("Resolved {} referenced nodes", node_lookup.len());

    let (mut polygons, skipped_polygons) = build_polygons(ways, &node_lookup);
    let (mut roads, skipped_roads) = build_road_polylines(road_ways, &node_lookup);
    ensure!(
        !polygons.is_empty() || !roads.is_empty(),
        "no valid terrain polygons or road polylines after node resolution"
    );

    println!(
        "Built {} terrain polygons (skipped {}) and {} road polylines (skipped {})",
        polygons.len(),
        skipped_polygons,
        roads.len(),
        skipped_roads,
    );

    let global_bounds = compute_global_bounds_for_features(&polygons, &roads)
        .ok_or_else(|| anyhow!("failed to compute polygon bounds"))?;

    let mercator_origin_x = (global_bounds.min_x / CHUNK_SIZE_METERS).floor() * CHUNK_SIZE_METERS;
    let mercator_origin_y = (global_bounds.min_y / CHUNK_SIZE_METERS).floor() * CHUNK_SIZE_METERS;

    for polygon in &mut polygons {
        for point in &mut polygon.points {
            point[0] = (point[0] - mercator_origin_x) * GIS_TO_WORLD_SCALE;
            point[1] = (point[1] - mercator_origin_y) * GIS_TO_WORLD_SCALE;
        }
    }

    for road in &mut roads {
        for point in &mut road.points {
            point[0] = (point[0] - mercator_origin_x) * GIS_TO_WORLD_SCALE;
            point[1] = (point[1] - mercator_origin_y) * GIS_TO_WORLD_SCALE;
        }
        road.width_m *= GIS_TO_WORLD_SCALE;
    }

    let local_bounds = compute_global_bounds_for_features(&polygons, &roads)
        .ok_or_else(|| anyhow!("failed to compute localized polygon bounds"))?;

    println!(
        "Indexing terrain and roads into chunk buckets ({} cells/chunk side)...",
        cells_per_side
    );
    let chunk_index = build_chunk_polygon_index(&polygons);
    let road_index = build_chunk_road_index(&roads);
    ensure!(
        !chunk_index.is_empty() || !road_index.is_empty(),
        "terrain and road rasterization produced no chunk buckets"
    );

    let min_chunk_x = if chunk_index.is_empty() {
        road_index.min_chunk_x
    } else if road_index.is_empty() {
        chunk_index.min_chunk_x
    } else {
        chunk_index.min_chunk_x.min(road_index.min_chunk_x)
    };
    let min_chunk_y = if chunk_index.is_empty() {
        road_index.min_chunk_y
    } else if road_index.is_empty() {
        chunk_index.min_chunk_y
    } else {
        chunk_index.min_chunk_y.min(road_index.min_chunk_y)
    };
    let max_chunk_x = if chunk_index.is_empty() {
        road_index.max_chunk_x
    } else if road_index.is_empty() {
        chunk_index.max_chunk_x
    } else {
        chunk_index.max_chunk_x.max(road_index.max_chunk_x)
    };
    let max_chunk_y = if chunk_index.is_empty() {
        road_index.max_chunk_y
    } else if road_index.is_empty() {
        chunk_index.max_chunk_y
    } else {
        chunk_index.max_chunk_y.max(road_index.max_chunk_y)
    };

    let pyramid_layout = compute_pyramid_layout(
        min_chunk_x,
        min_chunk_y,
        max_chunk_x,
        max_chunk_y,
    )?;

    println!(
        "Rasterizing polygons into streamed terrain chunks (fixed chunk buffer)..."
    );
    let output_path = world_output_path(region);
    let mut world_writer = WorldWriter::new(&output_path)?;

    let mut pyramid_stream = PyramidStreamReducer::new(
        pyramid_layout.playable_zoom_level,
        cells_per_side,
    )?;

    let mut parent_tile_count = 0_usize;

    let base_tile_count = stream_rasterized_chunks(
        &polygons,
        &chunk_index,
        &roads,
        &road_index,
        cells_per_side,
        raster_memory_budget_bytes,
        |chunk_x, chunk_y, cells| {
            let tile_x = chunk_x + pyramid_layout.playable_tile_offset_x;
            let tile_y = chunk_y + pyramid_layout.playable_tile_offset_y;

            if pyramid_layout.playable_zoom_level == 0 {
                world_writer.write_tile(pyramid_layout.playable_zoom_level, tile_x, tile_y, cells)?;
                return Ok(());
            }

            world_writer.write_tile_from_slice(
                pyramid_layout.playable_zoom_level,
                tile_x,
                tile_y,
                &cells,
            )?;

            parent_tile_count += pyramid_stream.push_playable_tile(
                tile_x,
                tile_y,
                cells,
                &mut |zoom, parent_x, parent_y, parent_cells| {
                    world_writer.write_tile_from_slice(zoom, parent_x, parent_y, parent_cells)
                },
            )?;

            Ok(())
        },
    )?;

    if pyramid_layout.playable_zoom_level > 0 {
        println!("Building zoom pyramid from streamed base chunks...");
        parent_tile_count += pyramid_stream.finish(&mut |zoom, tile_x, tile_y, cells| {
            world_writer.write_tile_from_slice(zoom, tile_x, tile_y, cells)
        })?;
    }

    let total_tile_count = base_tile_count + parent_tile_count;

    println!(
        "Generated {} total tiles across zoom levels 0..{}",
        total_tile_count,
        pyramid_layout.playable_zoom_level
    );
    println!(
        "Playable chunk size: {:.2}m (cell size: {:.2}m)",
        pyramid_layout.playable_tile_size_m,
        pyramid_layout.playable_tile_size_m / cells_per_side as f32,
    );

    let generated_unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock drifted before unix epoch")?
        .as_secs();

    let metadata = WorldMetadata {
        region: region.to_string(),
        source_pbf: raw_file_path.display().to_string(),
        generated_unix_seconds,
        chunk_size_m: pyramid_layout.playable_tile_size_m,
        cells_per_side: cells_per_side as u16,
        playable_zoom_level: pyramid_layout.playable_zoom_level,
        playable_tile_offset_x: pyramid_layout.playable_tile_offset_x,
        playable_tile_offset_y: pyramid_layout.playable_tile_offset_y,
        mercator_origin_x_m: mercator_origin_x,
        mercator_origin_y_m: mercator_origin_y,
        local_bounds_min_x: local_bounds.min_x as f32,
        local_bounds_min_y: local_bounds.min_y as f32,
        local_bounds_max_x: local_bounds.max_x as f32,
        local_bounds_max_y: local_bounds.max_y as f32,
        tile_count: 0,
        tiles: Vec::new(),
    };

    world_writer.finish(&output_path, metadata)?;

    println!("Wrote processed terrain world file: {:?}", output_path);
    Ok(())
}
