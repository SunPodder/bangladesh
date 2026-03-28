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

## 4. Map Scale & LOD Invariants
- Preserve `1 world unit = 1 meter` across generator and runtime.
- Keep `zoom 0` as full-map framing; do not hardcode viewport-only zoom baselines disconnected from map extents.
- Treat camera zoom and baked LOD selection as separate concerns: camera zoom is continuous, while LOD comes from archived tile layers and is selected from index metadata.
- Prefer the vector tile workflow in `src/bin/map_gen/`; do not reintroduce raster chunk baking, procedural repair passes, or `.world`-file generation.
- Keep tile outputs independent: `map_index.json` plus `tile_<id>.rkyv`, with all LODs bundled per tile.
- Quantize generated tile geometry to tile-local `i32` coordinates and keep runtime/world transforms derived from tile origin + quantization scale.
- Keep terrain tag classification aligned between ways and multipolygon relations. Buildings remain a dedicated layer, and Bangladesh-local terrain variants should stay centralized in `src/bin/map_gen/terrain_tag_filters.rs`.
- Keep road extraction limited to line `highway=*` ways (skip `area=yes`) with deterministic width/class mapping.
- Prefer third-party geometry/indexing libraries (`geo`, `rstar`, `rkyv`, `memmap2`) over custom spatial cleanup or LOD heuristics when extending the pipeline.

## 5. Map-Gen Concurrency Safety
- Parse the PBF into shared feature sets once, then parallelize per-tile assembly with Rayon.
- Keep tile writes deterministic at the file level; each worker should own its output tile file and avoid shared mutable output buffers.
- Use spatial indexes for tile assignment instead of ad hoc global bucketing maps when scaling generation work.
