use crate::grid::TileGrid;
use crate::types::ScanResult;
use anyhow::Result;
use bangladesh::shared::world::{
    BoundsMetadata, LatLonBoundsMetadata, MapIndex, TileGridMetadata, TileManifest, write_map_index,
};
use std::path::Path;

pub fn write_index(
    output_dir: &Path,
    region: &str,
    pbf_path: &Path,
    grid: TileGrid,
    scan: &ScanResult,
    mut tiles: Vec<TileManifest>,
) -> Result<()> {
    tiles.sort_by_key(|tile| tile.id);

    let index = MapIndex {
        version: 1,
        region: region.to_string(),
        source_pbf: pbf_path.display().to_string(),
        lod_count: scan.lods.len() as u8,
        lod_viewing_distances_m: scan.lods.iter().map(|lod| lod.viewing_distance_m).collect(),
        lod_simplification_tolerances_m: scan
            .lods
            .iter()
            .map(|lod| lod.simplify_tolerance_m)
            .collect(),
        quantization_scale: bangladesh::shared::world::QUANTIZATION_SCALE,
        tile_grid: TileGridMetadata {
            origin_x_m: grid.origin_x_m,
            origin_y_m: grid.origin_y_m,
            tile_size_m: grid.tile_size_m,
            cols: grid.cols,
            rows: grid.rows,
        },
        world_bounds_mercator: BoundsMetadata {
            min_x: scan.mercator_bounds.min_x,
            min_y: scan.mercator_bounds.min_y,
            max_x: scan.mercator_bounds.max_x,
            max_y: scan.mercator_bounds.max_y,
        },
        world_bounds_lat_lon: LatLonBoundsMetadata {
            min_lat: scan.lat_lon_bounds.min_lat,
            min_lon: scan.lat_lon_bounds.min_lon,
            max_lat: scan.lat_lon_bounds.max_lat,
            max_lon: scan.lat_lon_bounds.max_lon,
        },
        tiles,
    };

    write_map_index(&output_dir.join("map_index.json"), &index)
}
