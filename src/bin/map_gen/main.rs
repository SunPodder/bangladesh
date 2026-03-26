mod config;
mod constants;
mod download;
mod geometry;
mod process;
mod pyramid;
mod rasterize;
mod terrain_extract;
mod terrain_types;

use anyhow::Result;
use bangladesh::shared::world::map_assets_path;
use clap::Parser;
use config::{Args, get_config};
use download::download_map;
use process::process_terrain_world;
use std::fs;

fn main() -> Result<()> {
    let args = Args::parse();
    let config = get_config(&args.region)?;

    let map_dir = map_assets_path();
    fs::create_dir_all(&map_dir)?;

    let raw_file_path = map_dir.join(&config.filename);

    println!(
        "--- Bangladesh Map Data Builder: {} ---",
        args.region.to_uppercase()
    );

    if !raw_file_path.exists() || args.force {
        download_map(&config, &raw_file_path)?;
    } else {
        println!(
            "File {} already exists. Use --force to re-download.",
            config.filename
        );
    }

    println!("Parsing and processing terrain from {:?}", raw_file_path);
    process_terrain_world(&args.region, &raw_file_path)?;
    println!("Processing complete: {}", args.region);

    Ok(())
}
