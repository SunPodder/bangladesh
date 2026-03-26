use crate::config::RegionConfig;
use anyhow::{Result, anyhow};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

pub fn download_map(config: &RegionConfig, target_path: &Path) -> Result<()> {
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
        Err(anyhow!("Mirror returned status: {}", response.status()))
    }
}
