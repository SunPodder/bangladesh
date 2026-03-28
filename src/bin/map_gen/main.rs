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
use indicatif::{ProgressBar, ProgressStyle};
use rayon::ThreadPoolBuilder;
use rayon::prelude::*;
use scan::{GenerateArgs, scan_pbf};
use std::fs;
use std::sync::{Arc, Mutex};
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
    let pbf_path = args.pbf_path();
    let output_dir = args.output_dir();
    let thread_count = args
        .threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from));
    let pool = ThreadPoolBuilder::new().num_threads(thread_count).build()?;

    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create output dir {}", output_dir.display()))?;

    println!("Scanning {}...", pbf_path.display());
    let (grid, scan) = scan_pbf(&args)?;
    println!(
        "Grid: {} cols x {} rows, {} total tile(s), {} selected tile(s), ~{} chunk(s), {} LOD(s)",
        grid.cols,
        grid.rows,
        grid.cols * grid.rows,
        scan.selected_tile_ids.len(),
        scan.total_chunks_estimate,
        scan.lods.len()
    );

    let index_writer =
        index::IndexWriter::prepare(&output_dir, &args.region, &pbf_path, grid, &scan)?;
    let already_generated = index_writer.generated_tile_ids();
    println!(
        "Index initialized at {} with {} baked tile(s) already present",
        output_dir.join("map_index.json").display(),
        already_generated.len()
    );

    let selected_tiles = scan
        .selected_tile_ids
        .iter()
        .map(|tile_id| scan.tile_specs[*tile_id as usize])
        .collect::<Vec<_>>();

    println!("Parsing OSM entities once for shared tile generation...");
    let parsed = parse::parse_map_data(&pbf_path)?;
    println!(
        "Parsed {} areas, {} buildings, {} roads, {} POIs",
        parsed.areas.len(),
        parsed.buildings.len(),
        parsed.roads.len(),
        parsed.pois.len()
    );

    let spatial_index = SpatialIndex::build(&parsed);
    let progress = if args.progress {
        let progress = ProgressBar::new(selected_tiles.len() as u64);
        progress.set_style(
            ProgressStyle::with_template(
                "[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} tiles ({percent}%) eta {eta_precise}",
            )?
            .progress_chars("##-"),
        );
        Some(progress)
    } else {
        None
    };
    let index_writer = Arc::new(Mutex::new(index_writer));

    let results = pool.install(|| {
        selected_tiles
            .par_iter()
            .map(|tile_spec| {
                let tile = build_tile(*tile_spec, &scan.lods, &parsed, &spatial_index)?;
                let manifest = serialize::write_tile(&output_dir, *tile_spec, &tile)?;
                {
                    let mut writer = index_writer
                        .lock()
                        .map_err(|_| anyhow::anyhow!("map index writer lock poisoned"))?;
                    writer.record_tile(manifest)?;
                }
                if let Some(progress) = &progress {
                    progress.inc(1);
                }
                Ok::<_, anyhow::Error>(())
            })
            .collect::<Vec<_>>()
    });

    for result in results {
        result?;
    }

    if let Some(progress) = &progress {
        progress.finish_and_clear();
    }

    println!(
        "Wrote map bundle to {} using up to {} worker thread(s)",
        output_dir.display(),
        thread_count
    );
    Ok(())
}
