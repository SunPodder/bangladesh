use crate::constants::{CHUNK_SIZE_METERS, GIS_TO_WORLD_SCALE};
use crate::geometry::compute_global_bounds;
use crate::pyramid::{LevelSpoolWriter, build_parent_levels_from_spool, compute_pyramid_layout};
use crate::rasterize::{build_chunk_polygon_index, stream_rasterized_chunks};
use crate::terrain_extract::{build_polygons, collect_needed_nodes, collect_terrain_ways};
use anyhow::{Context, Result, anyhow, ensure};
use bangladesh::shared::world::{WorldMetadata, WorldWriter, world_output_path};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn process_terrain_world(
    region: &str,
    raw_file_path: &Path,
    cells_per_side: usize,
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

    println!("Scanning terrain ways from {:?}", raw_file_path);
    let (ways, needed_nodes) = collect_terrain_ways(raw_file_path)?;
    ensure!(
        !ways.is_empty(),
        "no terrain-compatible closed ways found in {:?}",
        raw_file_path
    );

    println!(
        "Collected {} terrain ways and {} required nodes",
        ways.len(),
        needed_nodes.len()
    );

    let node_lookup = collect_needed_nodes(raw_file_path, &needed_nodes)?;
    println!("Resolved {} referenced nodes", node_lookup.len());

    let (mut polygons, skipped_polygons) = build_polygons(ways, &node_lookup);
    ensure!(
        !polygons.is_empty(),
        "no valid polygons after node resolution"
    );

    println!(
        "Built {} terrain polygons (skipped {})",
        polygons.len(),
        skipped_polygons
    );

    let global_bounds = compute_global_bounds(&polygons)
        .ok_or_else(|| anyhow!("failed to compute polygon bounds"))?;

    let mercator_origin_x = (global_bounds.min_x / CHUNK_SIZE_METERS).floor() * CHUNK_SIZE_METERS;
    let mercator_origin_y = (global_bounds.min_y / CHUNK_SIZE_METERS).floor() * CHUNK_SIZE_METERS;

    for polygon in &mut polygons {
        for point in &mut polygon.points {
            point[0] = (point[0] - mercator_origin_x) * GIS_TO_WORLD_SCALE;
            point[1] = (point[1] - mercator_origin_y) * GIS_TO_WORLD_SCALE;
        }
    }

    let local_bounds = compute_global_bounds(&polygons)
        .ok_or_else(|| anyhow!("failed to compute localized polygon bounds"))?;

    println!(
        "Indexing polygon coverage into chunk buckets ({} cells/chunk side)...",
        cells_per_side
    );
    let chunk_index = build_chunk_polygon_index(&polygons);
    ensure!(
        !chunk_index.is_empty(),
        "terrain rasterization produced no chunk buckets"
    );
    let pyramid_layout = compute_pyramid_layout(
        chunk_index.min_chunk_x,
        chunk_index.min_chunk_y,
        chunk_index.max_chunk_x,
        chunk_index.max_chunk_y,
    )?;

    println!(
        "Rasterizing polygons into streamed terrain chunks (fixed chunk buffer)..."
    );
    let output_path = world_output_path(region);
    let output_dir = output_path
        .parent()
        .ok_or_else(|| anyhow!("failed to resolve world output directory"))?;
    let mut world_writer = WorldWriter::new(&output_path)?;

    let mut base_spool_writer = LevelSpoolWriter::create(
        output_dir,
        region,
        "base",
        pyramid_layout.playable_zoom_level,
        cells_per_side,
    )?;

    let base_tile_count = stream_rasterized_chunks(
        &polygons,
        &chunk_index,
        cells_per_side,
        |chunk_x, chunk_y, cells| {
            let tile_x = chunk_x + pyramid_layout.playable_tile_offset_x;
            let tile_y = chunk_y + pyramid_layout.playable_tile_offset_y;

            world_writer.write_tile_from_slice(
                pyramid_layout.playable_zoom_level,
                tile_x,
                tile_y,
                cells,
            )?;
            base_spool_writer.append_tile(tile_x, tile_y, cells)?;
            Ok(())
        },
    )?;

    println!("Building zoom pyramid from streamed base chunks...");
    let base_level_path = base_spool_writer.finish()?;
    let parent_tile_count = build_parent_levels_from_spool(
        &base_level_path,
        output_dir,
        region,
        cells_per_side,
        pyramid_layout.playable_zoom_level,
        |zoom, tile_x, tile_y, cells| world_writer.write_tile(zoom, tile_x, tile_y, cells),
    )?;
    let _ = std::fs::remove_file(&base_level_path);

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
