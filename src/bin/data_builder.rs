use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

const MIRRORS: &[&str] = &[
    "https://download.geofabrik.de/asia/bangladesh-latest.osm.pbf",
];

// Dedicated folders
const RAW_DATA_DIR: &str = "assets/data/raw";
const PROCESSED_DATA_DIR: &str = "assets/data/processed";
const OSM_FILE: &str = "bangladesh-latest.osm.pbf";

fn download_map() -> Result<()> {
    let raw_path = Path::new(RAW_DATA_DIR).join(OSM_FILE);
    
    for url in MIRRORS {
        println!("Attempting download from: {}", url);
        
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(1200)) // 20 min timeout
            .build()?;

        let mut response = client.get(*url).send()?;

        if response.status().is_success() {
            let total_size = response
                .content_length()
                .context("Failed to get content length from mirror")?;

            // Setup Progress Bar
            let pb = ProgressBar::new(total_size);
            pb.set_style(ProgressStyle::default_bar()
                .template("{msg}\n{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")?
                .progress_chars("#>-"));
            pb.set_message(format!("Downloading {}", OSM_FILE));

            let mut dest = File::create(&raw_path)?;
            let mut buffer = [0; 8192]; // 8KB chunks
            
            loop {
                let bytes_read = response.read(&mut buffer)?;
                if bytes_read == 0 { break; }
                dest.write_all(&buffer[..bytes_read])?;
                pb.inc(bytes_read as u64);
            }

            pb.finish_with_message("Download complete!");
            return Ok(());
        } else {
            println!("Mirror returned status: {}", response.status());
        }
    }
    Err(anyhow::anyhow!("All mirrors failed or were too slow."))
}

fn main() -> Result<()> {
    println!("--- Bangladesh Map Data Builder ---");
    
    // Ensure directories exist
    fs::create_dir_all(RAW_DATA_DIR)?;
    fs::create_dir_all(PROCESSED_DATA_DIR)?;
    
    let raw_file_path = Path::new(RAW_DATA_DIR).join(OSM_FILE);

    // Step 1: Download
    if !raw_file_path.exists() {
        download_map()?;
    } else {
        let metadata = fs::metadata(&raw_file_path)?;
        println!("File already exists in {} ({:.2} MB). Skipping download.", 
            RAW_DATA_DIR, metadata.len() as f64 / 1_000_000.0);
    }
    
    // Step 2 & 3: Parsing and Building
    // You can now point your parser to RAW_DATA_DIR and save to PROCESSED_DATA_DIR
    println!("Next: Parse data from {} and output to {}", RAW_DATA_DIR, PROCESSED_DATA_DIR);
    
    Ok(())
}
