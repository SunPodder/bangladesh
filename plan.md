# OSM PBF → Bevy 1:1 Rkyv Pipeline (CLI Generation)

## Goal
Standalone CLI tool for offline generation of country-wide OSM map into zero-copy, LOD-stratified rkyv tiles for Bevy 1:1 game engine. Power-user only; focused on correctness and performance, not polish. Tiles are fully independent and mmap-loadable.

## Context
- **Target Map**: Bangladesh (~147,570 km²)
- **In-Game Chunks**: 147,570 × 1000m chunks
- **Game Scale**: 1:1 (1 meter = 1 Bevy unit)
- **Generation Model**: Prebaked, offline; parallel CLI binary; tiles are independent
- **Tile Strategy**: 100km × 100km tiles (10,000 in-game chunks per rkyv file)
- **Generation Speed**: ~1 minute per tile (parallelizable; ~1.5–4 hours full country on 8-core)
- **Estimated Output**: ~147 rkyv tile files + 1 master index JSON
- **File Location**: `assets/map/{region}/*`

---

## Tile Chunking Strategy

### Why 100km × 100km?

| Metric | Value | Rationale |
|--------|-------|-----------|
| Tile size | 100km × 100km | 10,000 in-game chunks per file; true independence |
| Total tiles | ~147 | Minimal file count; fast parallel generation |
| Gen time per tile | ~1 minute | Proven in prototype; embarrassingly parallelizable |
| File size per tile | ~50–500 MB | Larger, but fewer files; still mmap-friendly |
| Complexity | Extremely low | Tiles don't need cross-boundary stitching; no index logic |
| User workflow | Fire and forget | Power users run once, walk away; full country in ~2–4 hours |

### Output Structure

**Master Index** (`map_index.json`):
```json
{
  "version": 1,
  "country": "Bangladesh",
  "lod_count": 7,
  "lod_viewing_distances": [50, 200, 1000, 5000, 20000, 100000, 500000],
  "lod_simplification_tolerances": [0.05, 0.2, 1.0, 5.0, 20.0, 100.0, 500.0],
  "tile_grid": {
    "origin_lat": 21.6,
    "origin_lon": 88.0,
    "tile_size_m": 100000,
    "cols": 5,
    "rows": 30
  },
  "tiles": [
    {
      "id": 0,
      "grid_x": 0,
      "grid_y": 0,
      "bounds_lat": [21.6, 22.6],
      "bounds_lon": [88.0, 89.0],
      "file": "tile_0.rkyv",
      "file_size_bytes": 234567890,
      "entity_counts": {"roads": 12450, "buildings": 28340, "pois": 890}
    }
  ]
}
```

---

## CLI Interface

### Command Structure

```bash
osm-map-gen generate \
  --pbf-path <path/to/data.pbf> \
  --output-dir <path/to/output> \
  --tile-size 100000 \
  [--bounds min_lat min_lon max_lat max_lon] \
  [--tile-ids 0-50,100-120] \
  [--threads <num>] \
  [--progress]
```

### Arguments
- `--pbf-path`: Input OSM PBF file (required)
- `--output-dir`: Output directory for tiles + index (required)
- `--tile-size`: Tile size in meters (default: 100000 = 100km)
- `--bounds`: Geographic bounding box `min_lat min_lon max_lat max_lon` (optional; generates only tiles within bounds)
- `--tile-ids`: Comma/dash-separated tile IDs or ranges (e.g., `0-50,100-120`; optional; generates only specified tiles)
- `--threads`: Parallel workers (default: num_cpus)
- `--progress`: Show progress bar per tile (default: off)

**LOD levels are auto-calculated** based on total input chunks (see below).

### Examples

Full country:
```bash
osm-map-gen generate \
  --pbf-path bangladesh.pbf \
  --output-dir ./tiles \
  --threads 8 \
  --progress
```

Dev iteration (single tile):
```bash
osm-map-gen generate \
  --pbf-path bangladesh.pbf \
  --output-dir ./test_tiles \
  --tile-ids 0
```

Dev iteration (range):
```bash
osm-map-gen generate \
  --pbf-path bangladesh.pbf \
  --output-dir ./test_tiles \
  --tile-ids 0-10
```

Geographic bounding box (Dhaka region):
```bash
osm-map-gen generate \
  --pbf-path bangladesh.pbf \
  --output-dir ./test_tiles \
  --bounds 23.6 90.3 23.8 90.5
```

City-only data:
```bash
osm-map-gen generate \
  --pbf-path dhaka_only.pbf \
  --output-dir ./dhaka_tiles \
  --threads 4
```

---

## Pipeline (Simplified)

### Stage 0: PBF Scan & LOD Calculation
**Input**: PBF file  
**Output**: Tile list + LOD count  
**Library**: `osmpbf`

1. Quick scan of PBF to determine geographic bounds
2. Calculate grid dimensions from bounds + tile size
3. Calculate dynamic LOD count using formula: `num_lods = max(2, ceil(log2(total_chunks / 1000)) + 1)`
4. If `--bounds` specified: convert to tile ID ranges
5. If `--tile-ids` specified: parse and validate ranges
6. If neither: generate all tiles
7. Output ordered list of tile IDs to process

---

### Stage 1: Parse & Quantize (per tile, parallel)
**Input**: Tile bounds (from resolved tile ID list)  
**Output**: Raw entity data (in-memory)  
**Library**: `osmpbf`

Per tile:
- Stream PBF, filter to tile bounds + 1% overlap margin (for topology)
- Extract: roads (classified), buildings, water, major POIs
- Tag normalization (highway→RoadType enum, etc.)
- **Quantize coords to i32** tile-relative (origin = SW corner, 0.01m resolution)
- Validate basic topology (no NaN, no degenerate geometries)

---

### Stage 2: Geometry Validation & Dynamic LOD Generation (per tile, parallel)
**Input**: Raw entity data  
**Output**: N LOD datasets (rkyv-ready structs)  
**Library**: `geo`

**Dynamic LOD Calculation**:
Before processing any tiles, scan the PBF to estimate total chunks. Calculate optimal LOD count:

```
total_chunks = pbf_bounds.area_m2 / (1000 * 1000)
num_lods = max(2, ceil(log2(total_chunks / 1000)) + 1)
```

Examples:
- City (10k chunks): `log2(10) + 1 = 4.3` → 4 LODs
- Large city (50k chunks): `log2(50) + 1 = 5.6` → 6 LODs
- Country (147k chunks): `log2(147) + 1 = 7.2` → 7 LODs
- Small region (1k chunks): `log2(1) + 1 = 1` → 2 LODs (clamped to min 2)

Per LOD, assign **viewing distance and simplification tolerance**:

```
LOD 0: distance ≤ 50m,    tolerance ε = 0.05m (full detail)
LOD 1: distance ≤ 200m,   tolerance ε = 0.2m
LOD 2: distance ≤ 1km,    tolerance ε = 1m
LOD 3: distance ≤ 5km,    tolerance ε = 5m
LOD 4: distance ≤ 20km,   tolerance ε = 20m
LOD 5: distance ≤ 100km,  tolerance ε = 100m
LOD 6: distance ≤ 500km,  tolerance ε = 500m
```

Per tile, for each LOD:
1. Apply `geo::SimplifyVW` with assigned tolerance
2. Spatial cull: remove entities below size thresholds (e.g., LOD 4 drops buildings < 100m²)
3. Aggregate: buildings → color zones, roads → simplified paths
4. Output rkyv-ready struct for this LOD

---

### Stage 3: Rkyv Serialization (per tile, parallel)
**Input**: N LOD datasets (N determined dynamically)  
**Output**: 1 rkyv file per tile (all LODs bundled)  
**Library**: `rkyv`

**Structure** (all LODs in single file):
```rust
#[derive(Archive, Serialize, Deserialize)]
pub struct TileData {
    pub version: u32,
    pub tile_id: u32,
    pub lod_count: u8,
    pub lods: ArchivedVec<LODData>,  // [LOD0, LOD1, ..., LOD_N]
}

#[derive(Archive, Serialize, Deserialize)]
pub struct LODData {
    pub lod_level: u8,
    pub vertices: ArchivedVec<Point3D<i32>>,
    pub roads: ArchivedVec<Road>,
    pub buildings: ArchivedVec<Building>,
    pub water: ArchivedVec<WaterBody>,
    pub pois: ArchivedVec<POI>,
    pub spatial_index: SerializedBVH,
}
```

**Output**: `tiles/tile_{id}.rkyv` (e.g., `tile_0.rkyv`, `tile_147.rkyv`; all LODs in one file)

---

### Stage 4: Index Generation (serial, after all tiles)
**Input**: All rkyv files + LOD metadata  
**Output**: `map_index.json`  
**Library**: `serde_json`

Walk output directory, collect:
- File sizes per tile
- Entity counts per tile
- LOD count (same for all tiles in a generation run)
- Viewing distances and simplification tolerances for each LOD

Generate single index JSON with full metadata.

---

## Implementation Strategy (Minimum Dev Time)

### Code Structure
```
osm-map-gen/
├── Cargo.toml
├── src/
│   ├── main.rs           # CLI + arg parsing (clap) + dispatch
│   ├── scan.rs           # Stage 0: PBF scan, bounds, LOD calc, tile ID resolution
│   ├── parse.rs          # Stage 1: osmpbf → entities
│   ├── validate.rs       # Stage 2: geo validation + dynamic LOD gen
│   ├── serialize.rs      # Stage 3: rkyv + io
│   ├── index.rs          # Stage 4: map_index.json generation
│   ├── types.rs          # All rkyv-serializable types
│   └── grid.rs           # Tile grid math (bounds ↔ tile IDs)
└── benches/              # (optional) micro-bench per tile
```

### Development Approach
1. **Reuse your prototype**: Wrap stages 1–3 with CLI + selective tile generation.
2. **Add Stage 0 (scan + LOD calc)**: ~100 lines; quick bounds scan + formula.
3. **Add tile ID resolution**: `--bounds` and `--tile-ids` parsing; grid math.
4. **Rayon for parallelism**: Spawn N workers, each processes one tile independently.
5. **No visualization**: Power users load tiles in-engine directly.
6. **Logging over metrics**: Use `env_logger` for debug output.

### Fast Path (1 week max)
```
Day 1: Wrap prototype + add clap CLI + tile ID resolution
Day 2: Add Stage 0 (PBF scan + LOD calc) + rayon parallelization
Day 3: Rkyv serialization + index generation
Day 4: Test selective generation (single tile, range, bounds) + verify round-trip in Bevy
Day 5: Profile + optimize hot path (likely PBF parsing or geo ops)
Day 5+: Full country build, buffer time
```

---

## Library Stack (Final, Minimal)

| Component | Library | Rationale |
|-----------|---------|-----------|
| CLI args | `clap` | Standard, 5-minute setup; derives or builder pattern |
| Parse OSM | `osmpbf` | Same as before, proven |
| Geometry | `geo` | Simplification + validation only |
| Serialize | `rkyv` | Zero-copy, deterministic |
| Config | `serde_json` | Index output |
| Parallelism | `rayon` | Dead simple: `.par_iter().for_each()` |
| Logging | `env_logger` (or none) | Optional; stderr is fine for CLI |
| Progress | `indicatif` | Optional, nice-to-have (2 lines of code) |

**Don't use**: petgraph (skip road graph; not needed for independent tiles), egui (no visualization), serde_yaml, nom, or other fancy parsers.

---

## Runtime Integration (Bevy)

### Asset Loading (unchanged from before)
```rust
// Bevy async asset loader
impl AssetLoader for TileLoader {
    fn load(bytes: Vec<u8>) -> Result<TileData> {
        let archived = rkyv::access::<TileData>(&bytes)?;
        Ok(archived.clone_inner())
    }
}

// Or zero-copy:
let archived = unsafe { rkyv::archived_root::<TileData>(&bytes) };
```

### Chunk Streaming
- Load LOD 0 for current tile + neighbors
- LOD 1 beyond 200m, LOD 2 beyond 1km, cull LOD 3
- Unload when out of range

### Procedural Fill
```rust
// Query spatial index
let nearby_buildings = tile.lods[0].spatial_index.locate_all_at_point(player_pos);
```

---

## Execution Workflow

### First Run (Full Country)
```bash
time osm-map-gen generate \
  --pbf-path bangladesh-latest.pbf \
  --output-dir ./map_data \
  --threads 8 \
  --progress

# Expected: 2–4 hours, produces 147 tiles + map_index.json
# Output: ~10–70 GB (depends on OSM density)
```

### Game Integration
Copy `map_data/` into game's asset directory. Game loads `map_index.json`, streams tiles on demand.

### Iteration (Dev Workflow)

**Single tile (2–3 seconds per LOD):**
```bash
osm-map-gen generate \
  --pbf-path bangladesh.pbf \
  --output-dir ./test_tiles \
  --tile-ids 0
```

**Tile range (30 seconds to 1 minute):**
```bash
osm-map-gen generate \
  --pbf-path bangladesh.pbf \
  --output-dir ./test_tiles \
  --tile-ids 0-10
```

**Geographic box—Dhaka region (~5 tiles, ~10 seconds):**
```bash
osm-map-gen generate \
  --pbf-path bangladesh.pbf \
  --output-dir ./test_tiles \
  --bounds 23.6 90.3 23.8 90.5
```

**City-only dataset** (auto-calculates fewer LODs):
```bash
osm-map-gen generate \
  --pbf-path dhaka_only.pbf \
  --output-dir ./dhaka_tiles \
  --threads 4
# LOD count = ceil(log2(city_chunks / 1000)) + 1
# For Dhaka (~5k chunks): ~3–4 LODs instead of 7
```

**Full country** (2–4 hours, parallelizable):
```bash
osm-map-gen generate \
  --pbf-path bangladesh.pbf \
  --output-dir ./full_tiles \
  --threads 8
```

---

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| **Dynamic LOD calculation** | Scales from city (3–4 LODs) to country (6–7 LODs); formula: `max(2, ceil(log2(chunks/1000))+1)` |
| **Selective generation** | `--tile-ids` and `--bounds` flags enable fast dev iteration; single tile in 2–3 seconds |
| **All LODs per file** | Simpler IO, one mmap per tile; game loads only needed LOD |
| **No cross-tile stitching** | 100km tiles are large enough; roads/buildings don't cross borders in practice |
| **Rayon, not tokio** | Tiles are CPU-bound (geometry ops); no async needed |
| **Stage 0 (PBF scan first)** | Determine LOD count once, apply consistently to all tiles |
| **Quantize i32 early** | Deterministic, rkyv-friendly, avoids float precision issues |
| **No QA stage** | Power users validate in-engine; save dev time |
| **One index JSON** | Simple, human-readable; game loads once at startup; includes LOD metadata |
| **Configurable input** | Same pipeline handles city PBF, region PBF, or country PBF; LODs auto-adjust |

---

## Estimated Timings (on 8-core i7)

| Scenario | Time | Notes |
|----------|------|-------|
| Full Bangladesh (147 tiles, parallel) | 2–4 hours | 7 LODs calculated; all tiles processed |
| Single tile generation | 2–3 seconds | Fast feedback loop for geometry testing |
| Tile range (0–10, 11 tiles) | 20–40 seconds | Selective dev iteration |
| Geographic bounds (small city, ~5 tiles) | ~10 seconds | Test specific region |
| Full Dhaka city only | 10–30 seconds | ~3–4 LODs (smaller dataset = fewer LODs) |
| Index generation (serial) | < 1 second | After all tiles complete |
| Game load index | < 100ms | Map metadata + LOD config |
| Mmap + deserialize single LOD | < 5ms | Per tile, zero-copy |

**Dev iteration loop**: Change geometry → regenerate 1 tile (2–3s) → reload in Bevy (instant).

---

## Success Criteria

- ✓ CLI generates full country in < 4 hours (parallelizable, no bottleneck)
- ✓ Selective generation enables single-tile iteration in 2–3 seconds (`--tile-ids` and `--bounds`)
- ✓ LOD count auto-scales: cities get 3–4 LODs, countries get 6–7 LODs
- ✓ Same pipeline handles city PBF, region PBF, or country PBF without config changes
- ✓ Tiles are independent (one file per tile; no cross-dependencies)
- ✓ Rkyv serialization is deterministic (bit-for-bit reproducible)
- ✓ Game loads tiles via mmap with zero-copy (< 5ms per LOD)
- ✓ No broken roads or impossible building shapes
- ✓ Players recognize their neighborhoods
