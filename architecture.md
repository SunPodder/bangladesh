# System Architecture

## World Scaling & GIS
- **Scale**: 1:1 (1 real-meter = 1 game unit).
- **Coordinate System**: WGS84 (Lat/Lon) projected to Web Mercator (EPSG:3857).
- **Chunking**: The map is divided into $1024m \times 1024m$ spatial chunks (about $1km^2$) for lazy loading.

## Data Pipeline
- **Ingestion**: `map_gen` fetches PBF extracts from Geofabrik/BBBike.
- **Processing (Terrain-first)**: `map_gen` parses terrain tags from both closed OSM ways and `type=multipolygon` relations (outer way members stitched into rings), resolves required node coordinates in a second pass, projects to Web Mercator, and rasterizes polygons to chunk-local terrain cells.
- **Urban Coverage Rule**: In terrain extraction, `building=*`, `amenity=*`, and `office=*` tags are treated as urban area signals (while higher-priority classes like water still win via terrain priority).
- **Terrain Tag Filters Module**: Tag-to-terrain filters and priority winner selection are centralized in `terrain_tag_filters.rs` and reused for both way and relation classification.
- **Local Tag Coverage**: Terrain filters explicitly include additional Bangladesh extract variants from `landuse=*`, `leisure=*`, and `natural=*` (for example `landuse=reservoir`, `landuse=slum`, `leisure=park`, `natural=scrub`) to reduce default-terrain fallback and preserve expected area semantics.
- **Default Fallback Diagnostics**: If no terrain filter matches but area-hint keys exist, extraction falls back to default terrain and logs the unmatched area-hint `key=value` tags.
- **Detail Resolution**: `map_gen` raster detail is configurable via `--cells-per-side` (even integer, default `256`), controlling max playable detail as $\text{cell size} = \frac{1024m}{\text{cells per side}}$.
- **Pyramid Bake**: `map_gen` derives a sparse hierarchical tile pyramid from real raster chunks only (`zoom = playable..0`) by 2x downsampling each parent from 4 children. No synthetic detail subdivision is generated.
- **Chunk Raster Windowing**: Rasterization now precomputes per-polygon chunk bounds, then processes bounded chunk-row windows in memory and emits finalized base tiles immediately; no global chunk->polygon map is retained.
- **Parallel Chunk Rasterization**: Chunk cell computation now runs in Rayon workers with per-chunk local buffers in bounded batches; tile emission remains single-threaded and ordered to keep world writes deterministic and race-free.
- **In-Memory Pyramid Streaming**: Parent LOD levels are now reduced from streamed base rows in-memory (row-pair reducers per level), and each finalized parent tile is emitted directly to the world writer.
- **Direct World Streaming Write**: `.world` generation now writes tile payloads directly to the final output file and appends metadata as a trailer pointer (world format v3), removing temporary tile spool files.
- **Memory Strategy**: Raster memory is controlled by `--raster-memory-gib` (default `8`) and window sizing, keeping peak usage near the requested budget while preserving deterministic output ordering.
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
