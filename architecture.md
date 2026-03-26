# System Architecture

## World Scaling & GIS
- **Scale**: 1:5 (1 real-meter = 0.2 game units).
- **Coordinate System**: WGS84 (Lat/Lon) projected to Web Mercator (EPSG:3857).
- **Chunking**: The map is divided into $1km \times 1km$ spatial chunks for lazy loading.

## Data Pipeline
- **Ingestion**: `map_gen` fetches PBF extracts from Geofabrik/BBBike.
- **Processing (Terrain-first)**: `map_gen` parses terrain-relevant closed OSM ways (water/forest/urban/farmland/sand/grass), resolves required node coordinates in a second pass, projects to Web Mercator, and rasterizes polygons to chunk-local terrain cells.
- **Pyramid Bake**: `map_gen` now derives a sparse hierarchical tile pyramid from playable chunks (`zoom = playable..0`) by 2x downsampling each parent from 4 children.
- **Storage**: Map assets are unified in `assets/map/`: source `.pbf` and processed `.world` files are separated by extension in the same directory.
- **Format**: `.world` stores compact metadata + tile index keyed by `(zoom, tile_x, tile_y)` + per-tile `rkyv` archived payloads.
- **Runtime Loading**: Metadata is loaded first; terrain tiles are loaded on-demand by camera zoom + visible bounds. Full world file must never be loaded all at once.

## Zoom LOD
- `playable_zoom_level` is the highest-detail terrain level (normal gameplay).
- Lower zooms are unplayable overview layers that progressively drop detail but keep continuity.
- Runtime converts camera scale -> zoom level and only keeps visible tiles (plus margin) resident.
- Pyramid downsampling now uses dominant-terrain voting per $2 \times 2$ sample window (with water-safe tie breaks) to prevent river/ocean classes from flooding land at overview zooms.

## Networking (Server-Authoritative)
- **Library**: `lightyear`.
- **Sync**: Components marked with `Replicate` are sent from Server -> Client.
- **Input**: Client-side prediction for vehicles; server-side reconciliation.
- **Headless**: Server runs with `minimal_plugins` (no rendering/window).

## Repository Map
- `src/main.rs`: Entry point. Handles CLI flags (`--server`, `--client`, `--host`).
- `src/bin/map_gen/`: Modular GIS pipeline binary (`main.rs` + focused submodules).
- `src/plugins/`: Core logic split into `shared`, `client`, and `server`.
- `assets/map/`: Raw `.pbf` inputs and processed `.world` outputs.
- `architecture.md`: High-level system design & networking specs.
- `agent.md`: Detailed rules for AI coding behavior.

## Core Tech
- **Engine**: Bevy (Rust) - ECS Architecture.
- **GIS**: `osmpbf`, `geo-types`.
- **Net**: `lightyear` (Server-authoritative).
- **CLI**: `clap` (derive), `indicatif` (progress).

## Runtime Debugging
- Press `F3` in runtime to toggle an on-screen debug HUD with player coordinates, cursor world coordinates, and current loaded zoom level.
