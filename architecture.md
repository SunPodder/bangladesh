# System Architecture

## World Scaling & GIS
- **Scale**: 1:1 (1 real-meter = 1 game unit).
- **Coordinate System**: WGS84 (Lat/Lon) projected to Web Mercator (EPSG:3857).
- **Chunking**: The map is divided into $1024m \times 1024m$ spatial chunks (about $1km^2$) for lazy loading.

## Data Pipeline
- **Ingestion**: `map_gen` fetches PBF extracts from Geofabrik/BBBike.
- **Processing (Terrain-first)**: `map_gen` parses terrain tags from both closed OSM ways and `type=multipolygon` relations (outer way members stitched into rings), resolves required node coordinates in a second pass, projects to Web Mercator, and rasterizes polygons to chunk-local terrain cells.
- **Detail Resolution**: `map_gen` raster detail is configurable via `--cells-per-side` (even integer, default `256`), controlling max playable detail as $\text{cell size} = \frac{1024m}{\text{cells per side}}$.
- **Pyramid Bake**: `map_gen` derives a sparse hierarchical tile pyramid from real raster chunks only (`zoom = playable..0`) by 2x downsampling each parent from 4 children. No synthetic detail subdivision is generated.
- **Chunk Raster Streaming**: Rasterization now builds a polygon->chunk index and then rasterizes one chunk at a time with a reusable fixed cell buffer; full base chunk-cell maps are no longer kept in memory.
- **Disk-Backed Pyramid Streaming**: Base tiles are spooled to a temporary level file, and parent levels are generated row-pair-at-a-time from that spool into the final world writer. Peak memory is bounded to row working sets instead of whole zoom levels.
- **Streaming World Write**: `map_gen` streams tiles into a temporary world tile-data spool and finalizes `.world` metadata afterward, avoiding in-memory accumulation of serialized tile bytes.
- **Memory Strategy**: Final world assembly uses a fixed reusable copy buffer when committing tile data to the output file, trading throughput for predictable memory bounds.
- **Storage**: Map assets are unified in `assets/map/`: source `.pbf` and processed `.world` files are separated by extension in the same directory.
- **Format**: `.world` stores compact metadata + tile index keyed by `(zoom, tile_x, tile_y)` + per-tile `rkyv` archived payloads.
- **Runtime Loading**: Metadata is loaded first; terrain tiles are loaded on-demand by camera zoom + visible bounds. Full world file must never be loaded all at once.

## Zoom LOD
- `playable_zoom_level` is the highest-detail terrain LOD available from real rasterized chunks.
- Lower zooms are overview layers that progressively drop detail while preserving topology continuity.
- Runtime camera zoom step `0` always fits the full map bounds on screen (with small padding) and uses LOD `0`.
- Runtime can zoom in beyond the number of LOD levels to reach playable human-scale framing; when this happens, terrain remains on max LOD.
- LOD selection is derived from camera scale and clamped to `0..playable_zoom_level`; only visible tiles (plus margin) stay resident.
- Runtime zoom input is continuous (synthetic camera zoom), while LOD switches live on-the-fly with hysteresis around scale thresholds to avoid flicker/chatter at boundaries.
- Playable camera framing is calibrated to target roughly `96m` visible across the viewport (street-level readability for a `1.8m` actor).
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
