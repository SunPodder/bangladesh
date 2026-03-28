mod geometry;
mod grid;
mod index;
mod parse;
mod scan;
mod serialize;
mod terrain_tag_filters;
mod types;
mod validate;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use indicatif::ProgressBar;
use rayon::ThreadPoolBuilder;
use rayon::prelude::*;
use scan::{GenerateArgs, scan_pbf};
use std::collections::HashSet;
use std::fs;
use validate::{SpatialIndex, build_tile};

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Offline OSM tile generator for Bangladesh RPG"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Generate(GenerateArgs),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Generate(args) => generate(args),
    }
}

fn generate(args: GenerateArgs) -> Result<()> {
    if let Some(threads) = args.threads {
        ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .ok();
    }

    fs::create_dir_all(&args.output_dir)
        .with_context(|| format!("failed to create output dir {}", args.output_dir.display()))?;
    clear_previous_outputs(&args.output_dir)?;

    println!("Scanning {}...", args.pbf_path.display());
    let (grid, scan) = scan_pbf(&args)?;
    println!(
        "Grid: {} cols x {} rows, {} selected tile(s), {} LOD(s)",
        grid.cols,
        grid.rows,
        scan.selected_tile_ids.len(),
        scan.lods.len()
    );

    println!("Parsing OSM entities once for shared tile generation...");
    let parsed = parse::parse_map_data(&args.pbf_path)?;
    println!(
        "Parsed {} areas, {} buildings, {} roads, {} POIs",
        parsed.areas.len(),
        parsed.buildings.len(),
        parsed.roads.len(),
        parsed.pois.len()
    );

    let spatial_index = SpatialIndex::build(&parsed);
    let selected = scan
        .selected_tile_ids
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let selected_tiles = scan
        .tile_specs
        .iter()
        .copied()
        .filter(|tile| selected.contains(&tile.id))
        .collect::<Vec<_>>();

    let progress = args
        .progress
        .then(|| ProgressBar::new(selected_tiles.len() as u64));

    let results = selected_tiles
        .par_iter()
        .map(|tile_spec| {
            let tile = build_tile(*tile_spec, &scan.lods, &parsed, &spatial_index)?;
            let manifest = serialize::write_tile(&args.output_dir, *tile_spec, &tile)?;
            if let Some(progress) = &progress {
                progress.inc(1);
            }
            Ok::<_, anyhow::Error>(manifest)
        })
        .collect::<Vec<_>>();

    if let Some(progress) = &progress {
        progress.finish_and_clear();
    }

    let mut manifests = Vec::with_capacity(results.len());
    for result in results {
        manifests.push(result?);
    }

    index::write_index(
        &args.output_dir,
        &args.region,
        &args.pbf_path,
        grid,
        &scan,
        manifests,
    )?;

    println!("Wrote map bundle to {}", args.output_dir.display());
    Ok(())
}

fn clear_previous_outputs(output_dir: &std::path::Path) -> Result<()> {
    for entry in fs::read_dir(output_dir)
        .with_context(|| format!("failed to read output dir {}", output_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let should_remove = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| {
                name == "map_index.json" || (name.starts_with("tile_") && name.ends_with(".rkyv"))
            })
            .unwrap_or(false);

        if should_remove {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove stale output {}", path.display()))?;
        }
    }

    Ok(())
}
