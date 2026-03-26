use anyhow::{Context, Result, anyhow, ensure};
use bangladesh::shared::world::{
    TerrainChunk, TerrainKind, WorldMetadata, world_output_path, write_world_file,
};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use osmpbf::{Element, ElementReader};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;
use std::fs;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const CHUNK_SIZE_METERS: f64 = 1024.0;
const CELLS_PER_SIDE: usize = 64;
const WEB_MERCATOR_MAX_LAT: f64 = 85.05112878;

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

#[derive(Clone)]
struct RawTerrainWay {
    terrain: TerrainKind,
    node_refs: Vec<i64>,
}

#[derive(Clone)]
struct TerrainPolygon {
    terrain: TerrainKind,
    points: Vec<[f64; 2]>,
}

#[derive(Clone, Copy)]
struct Bounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl Bounds {
    fn new(point: [f64; 2]) -> Self {
        Self {
            min_x: point[0],
            min_y: point[1],
            max_x: point[0],
            max_y: point[1],
        }
    }

    fn include(&mut self, point: [f64; 2]) {
        self.min_x = self.min_x.min(point[0]);
        self.min_y = self.min_y.min(point[1]);
        self.max_x = self.max_x.max(point[0]);
        self.max_y = self.max_y.max(point[1]);
    }

    fn include_bounds(&mut self, other: Bounds) {
        self.min_x = self.min_x.min(other.min_x);
        self.min_y = self.min_y.min(other.min_y);
        self.max_x = self.max_x.max(other.max_x);
        self.max_y = self.max_y.max(other.max_y);
    }
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

fn classify_way_terrain(way: &osmpbf::Way<'_>) -> Option<TerrainKind> {
    let mut best_match: Option<TerrainKind> = None;

    for (key, value) in way.tags() {
        let terrain = match (key, value) {
            ("natural", "water")
            | ("natural", "wetland")
            | ("natural", "bay")
            | ("natural", "coastline")
            | ("waterway", "riverbank") => Some(TerrainKind::Water),

            ("landuse", "forest") | ("natural", "wood") | ("landuse", "wood") => {
                Some(TerrainKind::Forest)
            }

            ("landuse", "residential")
            | ("landuse", "commercial")
            | ("landuse", "industrial")
            | ("landuse", "retail")
            | ("landuse", "construction") => Some(TerrainKind::Urban),

            ("landuse", "farmland")
            | ("landuse", "orchard")
            | ("landuse", "vineyard")
            | ("landuse", "greenhouse_horticulture")
            | ("landuse", "plant_nursery")
            | ("landuse", "plantation") => Some(TerrainKind::Farmland),

            ("natural", "sand") | ("natural", "beach") => Some(TerrainKind::Sand),

            ("landuse", "grass")
            | ("landuse", "meadow")
            | ("landuse", "village_green")
            | ("landuse", "recreation_ground")
            | ("natural", "grassland") => Some(TerrainKind::Grass),

            _ => None,
        };

        if let Some(candidate) = terrain {
            best_match = match best_match {
                Some(existing) if existing.priority() >= candidate.priority() => Some(existing),
                _ => Some(candidate),
            };
        }
    }

    best_match
}

fn collect_terrain_ways(
    pbf_path: &Path,
) -> Result<(Vec<RawTerrainWay>, HashSet<i64>)> {
    let mut ways = Vec::new();
    let mut needed_nodes = HashSet::new();

    ElementReader::from_path(pbf_path)
        .with_context(|| format!("failed to open pbf file {}", pbf_path.display()))?
        .for_each(|element| {
            let Element::Way(way) = element else {
                return;
            };

            let Some(terrain) = classify_way_terrain(&way) else {
                return;
            };

            let refs: Vec<i64> = way.refs().collect();
            if refs.len() < 4 || refs.first() != refs.last() {
                return;
            }

            needed_nodes.extend(refs.iter().copied());
            ways.push(RawTerrainWay {
                terrain,
                node_refs: refs,
            });
        })
        .context("failed during terrain-way scan")?;

    Ok((ways, needed_nodes))
}

fn lat_lon_to_web_mercator(lat: f64, lon: f64) -> [f64; 2] {
    let lat = lat.clamp(-WEB_MERCATOR_MAX_LAT, WEB_MERCATOR_MAX_LAT);
    let x = lon.to_radians() * 6_378_137.0;
    let y = ((PI / 4.0) + (lat.to_radians() / 2.0)).tan().ln() * 6_378_137.0;
    [x, y]
}

fn collect_needed_nodes(
    pbf_path: &Path,
    needed_nodes: &HashSet<i64>,
) -> Result<HashMap<i64, [f64; 2]>> {
    let mut node_lookup = HashMap::with_capacity(needed_nodes.len());

    ElementReader::from_path(pbf_path)
        .with_context(|| format!("failed to open pbf file {}", pbf_path.display()))?
        .for_each(|element| match element {
            Element::Node(node) => {
                if needed_nodes.contains(&node.id()) {
                    node_lookup
                        .entry(node.id())
                        .or_insert_with(|| lat_lon_to_web_mercator(node.lat(), node.lon()));
                }
            }
            Element::DenseNode(node) => {
                if needed_nodes.contains(&node.id()) {
                    node_lookup
                        .entry(node.id())
                        .or_insert_with(|| lat_lon_to_web_mercator(node.lat(), node.lon()));
                }
            }
            _ => {}
        })
        .context("failed during node scan")?;

    Ok(node_lookup)
}

fn build_polygons(
    ways: Vec<RawTerrainWay>,
    node_lookup: &HashMap<i64, [f64; 2]>,
) -> (Vec<TerrainPolygon>, usize) {
    let mut polygons = Vec::with_capacity(ways.len());
    let mut skipped = 0_usize;

    for way in ways {
        let mut points = Vec::with_capacity(way.node_refs.len());
        let mut missing_node = false;

        for node_id in way.node_refs {
            let Some(position) = node_lookup.get(&node_id) else {
                missing_node = true;
                break;
            };
            points.push(*position);
        }

        if missing_node || points.len() < 4 {
            skipped += 1;
            continue;
        }

        polygons.push(TerrainPolygon {
            terrain: way.terrain,
            points,
        });
    }

    (polygons, skipped)
}

fn polygon_bounds(points: &[[f64; 2]]) -> Bounds {
    let mut bounds = Bounds::new(points[0]);
    for point in points.iter().copied().skip(1) {
        bounds.include(point);
    }
    bounds
}

fn compute_global_bounds(polygons: &[TerrainPolygon]) -> Option<Bounds> {
    let first = polygons.first()?;
    let mut bounds = polygon_bounds(&first.points);

    for polygon in polygons.iter().skip(1) {
        bounds.include_bounds(polygon_bounds(&polygon.points));
    }

    Some(bounds)
}

fn point_in_polygon(point: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let mut j = polygon.len() - 1;

    for i in 0..polygon.len() {
        let xi = polygon[i][0];
        let yi = polygon[i][1];
        let xj = polygon[j][0];
        let yj = polygon[j][1];

        let intersects = ((yi > point[1]) != (yj > point[1]))
            && (point[0] < (xj - xi) * (point[1] - yi) / ((yj - yi) + 1e-12) + xi);

        if intersects {
            inside = !inside;
        }

        j = i;
    }

    inside
}

fn paint_polygon_into_chunks(
    polygon: &TerrainPolygon,
    chunk_cells: &mut HashMap<(i32, i32), Vec<u8>>,
) {
    let bounds = polygon_bounds(&polygon.points);
    let cell_size = CHUNK_SIZE_METERS / CELLS_PER_SIDE as f64;

    let min_chunk_x = (bounds.min_x / CHUNK_SIZE_METERS).floor() as i32;
    let max_chunk_x = (bounds.max_x / CHUNK_SIZE_METERS).floor() as i32;
    let min_chunk_y = (bounds.min_y / CHUNK_SIZE_METERS).floor() as i32;
    let max_chunk_y = (bounds.max_y / CHUNK_SIZE_METERS).floor() as i32;

    let terrain_code = polygon.terrain.code();
    let terrain_priority = polygon.terrain.priority();

    for chunk_y in min_chunk_y..=max_chunk_y {
        for chunk_x in min_chunk_x..=max_chunk_x {
            let chunk_origin_x = f64::from(chunk_x) * CHUNK_SIZE_METERS;
            let chunk_origin_y = f64::from(chunk_y) * CHUNK_SIZE_METERS;

            let min_ix =
                (((bounds.min_x - chunk_origin_x) / cell_size).floor() as i32).max(0);
            let max_ix =
                (((bounds.max_x - chunk_origin_x) / cell_size).ceil() as i32)
                    .min(CELLS_PER_SIDE as i32);
            let min_iy =
                (((bounds.min_y - chunk_origin_y) / cell_size).floor() as i32).max(0);
            let max_iy =
                (((bounds.max_y - chunk_origin_y) / cell_size).ceil() as i32)
                    .min(CELLS_PER_SIDE as i32);

            if min_ix >= max_ix || min_iy >= max_iy {
                continue;
            }

            let cells = chunk_cells
                .entry((chunk_x, chunk_y))
                .or_insert_with(|| vec![TerrainKind::Unknown.code(); CELLS_PER_SIDE * CELLS_PER_SIDE]);

            for iy in min_iy..max_iy {
                for ix in min_ix..max_ix {
                    let world_x = chunk_origin_x + (f64::from(ix) + 0.5) * cell_size;
                    let world_y = chunk_origin_y + (f64::from(iy) + 0.5) * cell_size;

                    if !point_in_polygon([world_x, world_y], &polygon.points) {
                        continue;
                    }

                    let index = (iy as usize) * CELLS_PER_SIDE + (ix as usize);
                    let existing_priority = TerrainKind::from_code(cells[index]).priority();
                    if terrain_priority >= existing_priority {
                        cells[index] = terrain_code;
                    }
                }
            }
        }
    }
}

fn merge_chunk_maps(
    left: &mut HashMap<(i32, i32), Vec<u8>>,
    right: HashMap<(i32, i32), Vec<u8>>,
) {
    for (chunk_key, right_cells) in right {
        let Some(left_cells) = left.get_mut(&chunk_key) else {
            left.insert(chunk_key, right_cells);
            continue;
        };

        for (left_cell, right_cell) in left_cells.iter_mut().zip(right_cells.iter()) {
            let right_priority = TerrainKind::from_code(*right_cell).priority();
            let left_priority = TerrainKind::from_code(*left_cell).priority();
            if right_priority > left_priority {
                *left_cell = *right_cell;
            }
        }
    }
}

fn rasterize_polygons(polygons: &[TerrainPolygon]) -> HashMap<(i32, i32), Vec<u8>> {
    polygons
        .par_iter()
        .fold(HashMap::new, |mut local_map, polygon| {
            paint_polygon_into_chunks(polygon, &mut local_map);
            local_map
        })
        .reduce(HashMap::new, |mut left, right| {
            merge_chunk_maps(&mut left, right);
            left
        })
}

fn process_terrain_world(region: &str, raw_file_path: &Path) -> Result<()> {
    println!("Scanning terrain ways from {:?}", raw_file_path);
    let (ways, needed_nodes) = collect_terrain_ways(raw_file_path)?;
    ensure!(
        !ways.is_empty(),
        "no terrain-compatible closed ways found in {:?}",
        raw_file_path
    );

    println!(
        "Collected {} terrain ways and {} required nodes",
        ways.len(),
        needed_nodes.len()
    );

    let node_lookup = collect_needed_nodes(raw_file_path, &needed_nodes)?;
    println!("Resolved {} referenced nodes", node_lookup.len());

    let (mut polygons, skipped_polygons) = build_polygons(ways, &node_lookup);
    ensure!(!polygons.is_empty(), "no valid polygons after node resolution");

    println!(
        "Built {} terrain polygons (skipped {})",
        polygons.len(),
        skipped_polygons
    );

    let global_bounds = compute_global_bounds(&polygons)
        .ok_or_else(|| anyhow!("failed to compute polygon bounds"))?;

    let mercator_origin_x = (global_bounds.min_x / CHUNK_SIZE_METERS).floor() * CHUNK_SIZE_METERS;
    let mercator_origin_y = (global_bounds.min_y / CHUNK_SIZE_METERS).floor() * CHUNK_SIZE_METERS;

    for polygon in &mut polygons {
        for point in &mut polygon.points {
            point[0] -= mercator_origin_x;
            point[1] -= mercator_origin_y;
        }
    }

    let local_bounds = compute_global_bounds(&polygons)
        .ok_or_else(|| anyhow!("failed to compute localized polygon bounds"))?;

    println!("Rasterizing polygons into chunked terrain grid (rayon enabled)...");
    let chunk_cells = rasterize_polygons(&polygons);
    ensure!(
        !chunk_cells.is_empty(),
        "terrain rasterization produced no chunk cells"
    );

    let mut chunks = Vec::with_capacity(chunk_cells.len());
    for ((chunk_x, chunk_y), cells) in chunk_cells {
        chunks.push(TerrainChunk {
            chunk_x,
            chunk_y,
            cells,
        });
    }
    chunks.sort_by_key(|chunk| (chunk.chunk_y, chunk.chunk_x));

    let generated_unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock drifted before unix epoch")?
        .as_secs();

    let metadata = WorldMetadata {
        region: region.to_string(),
        source_pbf: raw_file_path.display().to_string(),
        generated_unix_seconds,
        chunk_size_m: CHUNK_SIZE_METERS as f32,
        cells_per_side: CELLS_PER_SIDE as u16,
        mercator_origin_x_m: mercator_origin_x,
        mercator_origin_y_m: mercator_origin_y,
        local_bounds_min_x: local_bounds.min_x as f32,
        local_bounds_min_y: local_bounds.min_y as f32,
        local_bounds_max_x: local_bounds.max_x as f32,
        local_bounds_max_y: local_bounds.max_y as f32,
        chunk_count: 0,
        chunks: Vec::new(),
    };

    let output_path = world_output_path(region);
    write_world_file(&output_path, metadata, chunks)?;

    println!("Wrote processed terrain world file: {:?}", output_path);
    Ok(())
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

    println!("Parsing and processing terrain from {:?}", raw_file_path);
    process_terrain_world(&args.region, &raw_file_path)?;
    println!("Processing complete: {}", args.region);

    Ok(())
}
