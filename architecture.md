# System Architecture

## World Scaling & GIS
- **Scale**: 1:1 (1 real-meter = 1 game unit).
- **Coordinate System**: WGS84 (Lat/Lon) projected to Web Mercator (EPSG:3857).
- **Tile Strategy**: Offline map baking targets independent `100km x 100km` tiles for country-scale generation and streaming.

## Data Pipeline
- **Input Contract**: `map_gen generate` takes an explicit `.osm.pbf` path and writes a region bundle to `assets/map/{region}/` or another chosen output directory.
- **Stage 0 Scan**: The generator scans the PBF once for bounds, builds the tile grid, and derives LOD count with `max(2, ceil(log2(total_chunks / 1000)) + 1)`.
- **Shared Parse Pass**: OSM ways, multipolygon relations, and POI nodes are parsed once into reusable vector features; required node coordinates are resolved in a second pass and projected to Web Mercator.
- **Classification Rules**: `terrain_tag_filters.rs` remains the single source of truth for Bangladesh-specific terrain tags. Buildings are stored as a dedicated layer, and roads use explicit width/class buckets from `highway=*`.
- **Library-first Cleanup**: The previous raster/procedural refinement path has been removed. Geometry cleanup and LOD simplification now rely on `geo` validation, repeated-point removal, and topology-preserving Visvalingam-Whyatt simplification.
- **Spatial Assignment**: Features are assigned to overlapping tiles through `rstar` spatial indexes instead of custom chunk bucketing.
- **Storage Layout**: Each region bundle contains `map_index.json` plus independent `tile_<id>.rkyv` files.
- **Format**: `map_index.json` stores grid metadata, bounds, LOD thresholds, and per-tile manifests. Each tile archive stores vector areas, buildings, roads, and POIs quantized to tile-local `i32` coordinates.
- **Runtime Loading**: The Bevy runtime loads `map_index.json` first, mmaps only visible tile files, and renders the active LOD directly from archived tile data.

## Zoom LOD
- LOD count is data-dependent rather than fixed, so city extracts stay shallow while country extracts gain more overview levels automatically.
- Each tile bundles all LODs in one file; runtime zoom chooses which archived LOD to draw without changing file layout.
- LOD thresholds come from `map_index.json` viewing distances while the camera keeps continuous zoom.
- Runtime only keeps visible tiles plus a small preload margin resident.

## Networking (Server-Authoritative)
- **Library**: `lightyear`.
- **Sync**: Components marked with `Replicate` are sent from Server -> Client.
- **Input**: Client-side prediction for vehicles; server-side reconciliation.
- **Headless**: Server runs with `minimal_plugins` (no rendering/window).

## Repository Map
- `src/main.rs`: Entry point. Handles CLI flags (`--server`, `--client`, `--host`).
- `src/bin/map_gen/`: Vector-tile generation pipeline (`scan`, `parse`, `validate`, `serialize`, `index`, `grid`, `geometry`, `types`).
- `src/shared/world.rs`: Shared tile/index schema plus rkyv/json IO helpers.
- `src/shared/terrain_runtime.rs`: Runtime tile streaming, mmap loading, and gizmo rendering.
- `assets/map/`: Generated per-region map bundles (`map_index.json` + `tile_*.rkyv`).
- `architecture.md`: High-level system design & networking specs.
- `agent.md`: Detailed rules for AI coding behavior.

## Core Tech
- **Engine**: Bevy (Rust) - ECS Architecture.
- **GIS**: `osmpbf`, `geo`, `rstar`.
- **Serialization**: `rkyv`, `serde_json`, `memmap2`.
- **Net**: `lightyear` (Server-authoritative).
- **CLI**: `clap`, `indicatif`, `rayon`.

## Runtime Debugging
- Press `F3` in runtime to toggle an on-screen debug HUD with player coordinates, current scale, active LOD, and loaded tile count.
