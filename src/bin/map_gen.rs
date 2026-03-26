use anyhow::{Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path};

#[derive(Parser, Debug)]
#[command(author, version, about = "OSM Data Downloader & Processor")]
struct Args {
    /// The region to download (e.g., 'bangladesh', 'dhaka').
    /// If using a custom region, ensure the URL is added to the match logic.
    #[arg(short, long, default_value = "bangladesh")]
    region: String,

    /// Force re-download even if file exists
    #[arg(short, long)]
    force: bool,
}

struct RegionConfig {
    url: String,
    filename: String,
}

fn get_config(region: &str) -> Result<RegionConfig> {
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
        _ => Err(anyhow::anyhow!(
            "Unknown region: {}. Add it to get_config()",
            region
        )),
    }
}

fn download_map(config: &RegionConfig, target_path: &Path) -> Result<()> {
    println!("Attempting download from: {}", config.url);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(1200))
        .build()?;

    let mut response = client.get(&config.url).send()?;

    if response.status().is_success() {
        let total_size = response.content_length().unwrap_or(0);

        let pb = ProgressBar::new(total_size);
        pb.set_style(ProgressStyle::default_bar()
            .template("{msg}\n{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")?
            .progress_chars("#>-"));
        pb.set_message(format!("Downloading {}", config.filename));

        let mut dest = File::create(target_path)?;
        let mut buffer = [0; 8192];

        loop {
            let bytes_read = response.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            dest.write_all(&buffer[..bytes_read])?;
            pb.inc(bytes_read as u64);
        }

        pb.finish_with_message("Download complete!");
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "Mirror returned status: {}",
            response.status()
        ))
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let config = get_config(&args.region)?;

    let raw_dir = Path::new("assets/data/raw");
    let processed_dir = Path::new("assets/data/processed");
    fs::create_dir_all(raw_dir)?;
    fs::create_dir_all(processed_dir)?;

    let raw_file_path = raw_dir.join(&config.filename);

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

    println!("Ready to process: {:?}", raw_file_path);
    // parse_osm_data(&raw_file_path, processed_dir)?;

    Ok(())
}
