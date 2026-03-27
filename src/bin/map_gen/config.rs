use crate::constants::DEFAULT_CELLS_PER_SIDE;
use anyhow::{Result, anyhow};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about = "OSM Data Downloader & Processor")]
pub struct Args {
    /// The region to download (e.g., 'bangladesh', 'dhaka').
    /// If using a custom region, ensure the URL is added to the match logic.
    #[arg(short, long, default_value = "bangladesh")]
    pub region: String,

    /// Force re-download even if file exists
    #[arg(short, long)]
    pub force: bool,

    /// Terrain raster resolution per chunk side. Higher values improve detail but increase bake time and world size.
    #[arg(long, default_value_t = DEFAULT_CELLS_PER_SIDE)]
    pub cells_per_side: usize,
}

pub struct RegionConfig {
    pub url: String,
    pub filename: String,
}

pub fn get_config(region: &str) -> Result<RegionConfig> {
    match region.to_lowercase().as_str() {
        "bangladesh" => Ok(RegionConfig {
            url: "https://download.geofabrik.de/asia/bangladesh-latest.osm.pbf".to_string(),
            filename: "bangladesh.pbf".to_string(),
        }),
        // Add smaller test extracts here (BBBike is great for city-level extracts)
        "dhaka" => Ok(RegionConfig {
            url: "https://download.bbbike.org/osm/bbbike/Dhaka/Dhaka.osm.pbf".to_string(),
            filename: "dhaka.pbf".to_string(),
        }),
        _ => Err(anyhow!(
            "Unknown region: {}. Add it to get_config()",
            region
        )),
    }
}
