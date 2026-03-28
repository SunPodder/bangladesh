use crate::types::TileSpec;
use anyhow::{Context, Result};
use bangladesh::shared::world::{
    EntityCounts, MappedTile, TileData, TileManifest, tile_file_name, write_tile_file,
};
use std::path::Path;

pub fn write_tile(output_dir: &Path, tile_spec: TileSpec, tile: &TileData) -> Result<TileManifest> {
    let file_name = tile_file_name(tile_spec.id);
    let path = output_dir.join(&file_name);
    write_tile_file(&path, tile)?;
    let file_size_bytes = std::fs::metadata(&path)
        .with_context(|| format!("failed to stat tile file {}", path.display()))?
        .len();

    Ok(build_manifest(tile_spec, file_name, file_size_bytes, tile))
}

pub fn recover_tile(output_dir: &Path, tile_spec: TileSpec) -> Result<Option<TileManifest>> {
    let file_name = tile_file_name(tile_spec.id);
    let path = output_dir.join(&file_name);
    if !path.is_file() {
        return Ok(None);
    }

    let file_size_bytes = std::fs::metadata(&path)
        .with_context(|| format!("failed to stat tile file {}", path.display()))?
        .len();
    let mapped = MappedTile::open(&path)?;
    let tile = mapped.archived()?;
    let highest_detail = tile.lods.first();
    let counts = highest_detail
        .map(|lod| EntityCounts {
            roads: lod.roads.len(),
            buildings: lod.buildings.len(),
            areas: lod.areas.len(),
            pois: lod.pois.len(),
        })
        .unwrap_or_default();

    Ok(Some(TileManifest {
        id: tile_spec.id,
        grid_x: tile_spec.grid_x,
        grid_y: tile_spec.grid_y,
        file: file_name,
        file_size_bytes,
        origin_x_m: tile.origin_x_m.into(),
        origin_y_m: tile.origin_y_m.into(),
        entity_counts: counts,
    }))
}

fn build_manifest(
    tile_spec: TileSpec,
    file_name: String,
    file_size_bytes: u64,
    tile: &TileData,
) -> TileManifest {
    let highest_detail = tile.lods.first();
    let counts = highest_detail
        .map(|lod| EntityCounts {
            roads: lod.roads.len(),
            buildings: lod.buildings.len(),
            areas: lod.areas.len(),
            pois: lod.pois.len(),
        })
        .unwrap_or_default();

    TileManifest {
        id: tile_spec.id,
        grid_x: tile_spec.grid_x,
        grid_y: tile_spec.grid_y,
        file: file_name,
        file_size_bytes,
        origin_x_m: tile.origin_x_m,
        origin_y_m: tile.origin_y_m,
        entity_counts: counts,
    }
}
