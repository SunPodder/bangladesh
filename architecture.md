# System Architecture

## World Scaling & GIS
- **Scale**: 1:5 (1 real-meter = 0.2 game units).
- **Coordinate System**: WGS84 (Lat/Lon) projected to Web Mercator (EPSG:3857).
- **Chunking**: The map is divided into $1km \times 1km$ spatial chunks for lazy loading.

## Data Pipeline
- **Ingestion**: `build.rs` fetches PBF from Geofabrik/BBBike.
- **Processing**: `map_gen` binary filters OSM tags:
    - `building`: Prefilled structures (colliders).
    - `highway`: Vehicle navigation paths.
    - `landuse=grass/meadow`: Player build-zones.
- **Storage**: Processed data is stored in `bincode` format for zero-cost deserialization.

## Networking (Server-Authoritative)
- **Library**: `lightyear`.
- **Sync**: Components marked with `Replicate` are sent from Server -> Client.
- **Input**: Client-side prediction for vehicles; server-side reconciliation.
- **Headless**: Server runs with `minimal_plugins` (no rendering/window).

## Repository Map
- `src/main.rs`: Entry point. Handles CLI flags (`--server`, `--client`, `--host`).
- `src/bin/map_gen.rs`: GIS Data Pipeline. Converts `.pbf` to game binary.
- `src/plugins/`: Core logic split into `shared`, `client`, and `server`.
- `assets/data/raw/`: Raw `.osm.pbf` files (ignored by git).
- `assets/data/processed/`: Game-ready binary chunks.
- `architecture.md`: High-level system design & networking specs.
- `agent.md`: Detailed rules for AI coding behavior.

## Core Tech
- **Engine**: Bevy (Rust) - ECS Architecture.
- **GIS**: `osmpbf`, `geo-types`.
- **Net**: `lightyear` (Server-authoritative).
- **CLI**: `clap` (derive), `indicatif` (progress).
