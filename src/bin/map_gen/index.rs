use crate::grid::TileGrid;
use crate::serialize;
use crate::types::{ScanResult, TileSpec};
use anyhow::Result;
use bangladesh::shared::world::{
    BoundsMetadata, LatLonBoundsMetadata, MapIndex, TileGridMetadata, TileManifest, read_map_index,
    write_map_index,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct IndexWriter {
    path: PathBuf,
    index: MapIndex,
}

impl IndexWriter {
    pub fn prepare(
        output_dir: &Path,
        region: &str,
        pbf_path: &Path,
        grid: TileGrid,
        scan: &ScanResult,
    ) -> Result<Self> {
        let path = output_dir.join("map_index.json");
        let index = build_index(output_dir, &path, region, pbf_path, grid, scan)?;
        write_map_index(&path, &index)?;
        Ok(Self { path, index })
    }

    pub fn record_tile(&mut self, manifest: TileManifest) -> Result<()> {
        upsert_manifest(&mut self.index.tiles, manifest);
        write_map_index(&self.path, &self.index)
    }

    pub fn generated_tile_ids(&self) -> HashSet<u32> {
        self.index.tiles.iter().map(|tile| tile.id).collect()
    }
}

fn build_index(
    output_dir: &Path,
    index_path: &Path,
    region: &str,
    pbf_path: &Path,
    grid: TileGrid,
    scan: &ScanResult,
) -> Result<MapIndex> {
    let existing = read_map_index(index_path).ok();
    let mut tiles = Vec::new();

    for tile_spec in &scan.tile_specs {
        if let Some(manifest) = recover_manifest(output_dir, *tile_spec, existing.as_ref())? {
            tiles.push(manifest);
        }
    }
    tiles.sort_by_key(|tile| tile.id);

    Ok(MapIndex {
        version: 1,
        region: region.to_string(),
        source_pbf: pbf_path.display().to_string(),
        total_chunks_estimate: scan.total_chunks_estimate,
        total_tile_count: grid.cols * grid.rows,
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
    })
}

fn recover_manifest(
    output_dir: &Path,
    tile_spec: TileSpec,
    existing: Option<&MapIndex>,
) -> Result<Option<TileManifest>> {
    if let Some(manifest) = serialize::recover_tile(output_dir, tile_spec)? {
        return Ok(Some(manifest));
    }

    Ok(existing.and_then(|index| {
        index
            .tiles
            .iter()
            .find(|tile| tile.id == tile_spec.id)
            .cloned()
            .filter(|tile| output_dir.join(&tile.file).is_file())
    }))
}

fn upsert_manifest(tiles: &mut Vec<TileManifest>, manifest: TileManifest) {
    match tiles.binary_search_by_key(&manifest.id, |tile| tile.id) {
        Ok(index) => tiles[index] = manifest,
        Err(index) => tiles.insert(index, manifest),
    }
}
