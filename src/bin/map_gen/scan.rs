use crate::geometry::{Bounds, LatLonBounds, lat_lon_to_web_mercator};
use crate::grid::{TileGrid, parse_tile_id_ranges};
use crate::types::{LodSettings, ScanResult};
use anyhow::{Context, Result, ensure};
use bangladesh::shared::world::region_map_path;
use clap::Args;
use osmpbf::{Element, ElementReader};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Args)]
pub struct GenerateArgs {
    #[arg(long)]
    pub region: String,

    #[arg(long, default_value_t = 100_000)]
    pub tile_size: u32,

    #[arg(long, num_args = 4)]
    pub bounds: Option<Vec<f64>>,

    #[arg(long)]
    pub tile_ids: Option<String>,

    #[arg(long)]
    pub threads: Option<usize>,

    #[arg(long, default_value_t = false)]
    pub progress: bool,
}

impl GenerateArgs {
    pub fn pbf_path(&self) -> PathBuf {
        region_map_path(&self.region).join(format!("{}.pbf", self.region))
    }

    pub fn output_dir(&self) -> PathBuf {
        region_map_path(&self.region)
    }
}

pub fn scan_pbf(args: &GenerateArgs) -> Result<(TileGrid, ScanResult)> {
    let pbf_path = args.pbf_path();
    let (mercator_bounds, lat_lon_bounds) = scan_bounds(&pbf_path)?;
    let grid = TileGrid::from_bounds(mercator_bounds, args.tile_size)?;
    let tile_specs = grid.tile_specs();
    let total_chunks = (mercator_bounds.area() / 1_000_000.0).max(1.0).ceil() as u64;
    let lod_count = (((total_chunks as f64) / 1000.0).log2().ceil() as i32 + 1).max(2) as usize;

    let default_distances = [
        50.0_f32, 200.0, 1_000.0, 5_000.0, 20_000.0, 100_000.0, 500_000.0,
    ];
    let default_tolerances = [0.05_f32, 0.2, 1.0, 5.0, 20.0, 100.0, 500.0];

    let lods = (0..lod_count)
        .map(|index| LodSettings {
            viewing_distance_m: *default_distances
                .get(index)
                .unwrap_or(default_distances.last().unwrap()),
            simplify_tolerance_m: *default_tolerances
                .get(index)
                .unwrap_or(default_tolerances.last().unwrap()),
        })
        .collect::<Vec<_>>();

    let max_tile_id = tile_specs.last().map(|tile| tile.id).unwrap_or(0);
    let mut selected_tile_ids = if let Some(tile_ids) = &args.tile_ids {
        parse_tile_id_ranges(tile_ids, max_tile_id)?
    } else if let Some(bounds) = &args.bounds {
        ensure!(
            bounds.len() == 4,
            "--bounds requires min_lat min_lon max_lat max_lon"
        );
        grid.tile_ids_for_lat_lon_bounds(bounds[0], bounds[1], bounds[2], bounds[3])?
    } else {
        tile_specs.iter().map(|tile| tile.id).collect()
    };

    selected_tile_ids.sort_unstable();
    selected_tile_ids.dedup();

    Ok((
        grid,
        ScanResult {
            mercator_bounds,
            lat_lon_bounds,
            total_chunks_estimate: total_chunks,
            tile_specs,
            selected_tile_ids,
            lods,
        },
    ))
}

fn scan_bounds(path: &Path) -> Result<(Bounds, LatLonBounds)> {
    let mut mercator_bounds: Option<Bounds> = None;
    let mut lat_lon_bounds: Option<LatLonBounds> = None;

    ElementReader::from_path(path)
        .with_context(|| format!("failed to open pbf file {}", path.display()))?
        .for_each(|element| match element {
            Element::Node(node) => update_bounds(
                &mut mercator_bounds,
                &mut lat_lon_bounds,
                node.lat(),
                node.lon(),
            ),
            Element::DenseNode(node) => update_bounds(
                &mut mercator_bounds,
                &mut lat_lon_bounds,
                node.lat(),
                node.lon(),
            ),
            _ => {}
        })
        .context("failed while scanning pbf bounds")?;

    let mercator_bounds = mercator_bounds.context("pbf contained no node coordinates")?;
    let lat_lon_bounds = lat_lon_bounds.context("pbf contained no lat/lon coordinates")?;
    Ok((mercator_bounds, lat_lon_bounds))
}

fn update_bounds(
    mercator_bounds: &mut Option<Bounds>,
    lat_lon_bounds: &mut Option<LatLonBounds>,
    lat: f64,
    lon: f64,
) {
    let mercator = lat_lon_to_web_mercator(lat, lon);
    match mercator_bounds {
        Some(bounds) => bounds.include(mercator),
        None => *mercator_bounds = Some(Bounds::new(mercator)),
    }

    match lat_lon_bounds {
        Some(bounds) => bounds.include(lat, lon),
        None => *lat_lon_bounds = Some(LatLonBounds::new(lat, lon)),
    }
}
