# Agent Instructions & Coding Standards

## 1. Rust Patterns
- **Errors**: Prefer `anyhow` for application logic and `thiserror` for library crates.
- **Async**: Use `reqwest` for I/O; avoid async in Bevy systems (use `IoTaskPool` instead).
- **Types**: Use `glam` types (Vec2, Vec3) as provided by Bevy.

## 2. Bevy ECS Standards
- **Systems**: Keep them small and focused.
- **Queries**: Use `With<T>` and `Without<T>` filters to keep queries performant.
- **States**: Use `OnEnter(GameState::Playing)` for setup logic.

## 3. Multiplayer Standards
- Always assume the server has the source of truth.
- Local movement should use `lightyear`'s prediction components to eliminate perceived lag.
- Use `FixedUpdate` for physics and gameplay logic to ensure deterministic behavior across different frame rates.

## 4. Map Scale & Zoom Invariants
- Preserve `1 world unit = 1 meter` across generator and runtime.
- Keep `zoom 0` as full-map framing; do not hardcode viewport-only zoom baselines disconnected from map extents.
- Treat terrain LOD depth and camera zoom depth as separate concerns: LOD comes from baked tiles, camera zoom may go further while clamped to max LOD.
- Prefer continuous camera zoom input with hysteresis-based LOD thresholding to keep zoom smooth while preventing rapid LOD thrashing.
- Avoid synthetic tile subdivision that duplicates raster data; increase geometric detail through better source extraction or higher `--cells-per-side` instead.
- For large map extracts, keep world generation streaming-first: do not collect the full tile pyramid or serialized world bytes in memory before writing.
- Keep base raster generation chunk-streamed as well: avoid retaining `HashMap<(chunk_x, chunk_y), Vec<u8>>` for the whole map when processing large extracts.
- Prefer bounded in-memory row/window reducers over temporary spool files for pyramid/raster stages; keep tile emission ordered by `(tile_y, tile_x)` for deterministic output.
- Respect `--raster-memory-gib` as the raster window budget control when tuning large-area generation stability.
- Keep terrain tag classification aligned between ways and multipolygon relations; currently `building=*`, `amenity=*`, and `office=*` are treated as `Urban` hints unless a higher-priority terrain tag (for example `natural=water`) overrides.
- Keep road extraction limited to line `highway=*` ways (skip `area=yes` roads), with deterministic width-class mapping and raster paint after terrain so `TerrainKind::Road` remains visible as an overlay.
- Keep Bangladesh-local `landuse=*`, `leisure=*`, and `natural=*` variants in the terrain filters when unmatched fallback logs identify recurring values (for example `reservoir`, `slum`, `park`, `scrub`).
- Keep tag-to-terrain mapping and best-match priority logic in `src/bin/map_gen/terrain_tag_filters.rs`; when default terrain fallback triggers, preserve logging of unmatched area-hint tags for filter expansion work.
- In pyramid downsampling tie cases, preserve water only when a 2-2 tie forms an edge-connected strip (row/column) so major rivers remain visible in zoom 0 without letting diagonal/noisy water speckles spread.
- In pyramid downsampling tie cases, preserve roads only when a 2-2 tie forms an edge-connected strip (row/column) so major corridors remain visible in zoom 0 without letting diagonal/speckle road noise spread.
- For small overview LODs (zoom `0..2`), keep roads corridor-like: apply deterministic post-downsample cleanup to remove orphan/island road speckles and erode dense road blobs while preserving continuous major guidance lines.
- Keep GIS as the macro source of truth, but allow deterministic per-chunk procedural continuity repair after rasterization (bridge 1-cell road/water breaks, trim short dangling road stubs, and keep river-like water components flowing); preserve chunk-edge segments to avoid cutting cross-chunk continuity.

## 5. Map-Gen Concurrency Safety
- Parallelize terrain chunk cell computation with Rayon only when each worker writes to a chunk-local buffer.
- Keep final tile emission and world/spool writes single-threaded and deterministic (sorted chunk order).
- Use bounded batch sizes for parallel compute stages before sequential IO emission to prevent unbounded memory growth.

