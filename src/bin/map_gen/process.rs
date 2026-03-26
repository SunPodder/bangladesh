use crate::constants::{CELLS_PER_SIDE, CHUNK_SIZE_METERS};
use crate::geometry::compute_global_bounds;
use crate::pyramid::generate_tile_pyramid;
use crate::rasterize::rasterize_polygons;
use crate::terrain_extract::{build_polygons, collect_needed_nodes, collect_terrain_ways};
use anyhow::{Context, Result, anyhow, ensure};
use bangladesh::shared::world::{WorldMetadata, world_output_path, write_world_file};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn process_terrain_world(region: &str, raw_file_path: &Path) -> Result<()> {
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
    ensure!(!polygons.is_empty(), "no valid polygons after node resolution");

    println!(
        "Built {} terrain polygons (skipped {})",
        polygons.len(),
        skipped_polygons
    );

    let global_bounds =
        compute_global_bounds(&polygons).ok_or_else(|| anyhow!("failed to compute polygon bounds"))?;

    let mercator_origin_x = (global_bounds.min_x / CHUNK_SIZE_METERS).floor() * CHUNK_SIZE_METERS;
    let mercator_origin_y = (global_bounds.min_y / CHUNK_SIZE_METERS).floor() * CHUNK_SIZE_METERS;

    for polygon in &mut polygons {
        for point in &mut polygon.points {
            point[0] -= mercator_origin_x;
            point[1] -= mercator_origin_y;
        }
    }

    let local_bounds = compute_global_bounds(&polygons)
        .ok_or_else(|| anyhow!("failed to compute localized polygon bounds"))?;

    println!("Rasterizing polygons into chunked terrain grid (rayon enabled)...");
    let chunk_cells = rasterize_polygons(&polygons);
    ensure!(
        !chunk_cells.is_empty(),
        "terrain rasterization produced no chunk cells"
    );

    println!("Building zoom pyramid tiles from playable chunks...");
    let (tiles, playable_zoom_level, playable_tile_offset_x, playable_tile_offset_y) =
        generate_tile_pyramid(chunk_cells)?;

    println!(
        "Generated {} total tiles across zoom levels 0..{}",
        tiles.len(),
        playable_zoom_level
    );

    let generated_unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock drifted before unix epoch")?
        .as_secs();

    let metadata = WorldMetadata {
        region: region.to_string(),
        source_pbf: raw_file_path.display().to_string(),
        generated_unix_seconds,
        chunk_size_m: CHUNK_SIZE_METERS as f32,
        cells_per_side: CELLS_PER_SIDE as u16,
        playable_zoom_level,
        playable_tile_offset_x,
        playable_tile_offset_y,
        mercator_origin_x_m: mercator_origin_x,
        mercator_origin_y_m: mercator_origin_y,
        local_bounds_min_x: local_bounds.min_x as f32,
        local_bounds_min_y: local_bounds.min_y as f32,
        local_bounds_max_x: local_bounds.max_x as f32,
        local_bounds_max_y: local_bounds.max_y as f32,
        tile_count: 0,
        tiles: Vec::new(),
    };

    let output_path = world_output_path(region);
    write_world_file(&output_path, metadata, tiles)?;

    println!("Wrote processed terrain world file: {:?}", output_path);
    Ok(())
}
