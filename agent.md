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

## 4. Conversation Decisions
- 2026-03-26: Implement terrain-only world processing first: parse OSM terrain polygons, archive chunk payloads with `rkyv`, write a single `{region}.world`, and stream chunk loading in Bevy strictly by player position.
- 2026-03-26: Upgrade terrain world format to a multi-zoom tile pyramid `(zoom, x, y)` with playable zoom offsets, bake overview layers during `map_gen`, and stream tiles by camera zoom + viewport for smooth zoom-out to full-map view.
- 2026-03-26: Refactor `map_gen` into modular submodules under `src/bin/map_gen/` to keep GIS processing maintainable while preserving ECS-friendly data boundaries.
- 2026-03-26: Standardize map asset storage to `assets/map/` with `.pbf` and `.world` separated by extension, and default unresolved terrain areas to grass during bake/downsampling.
- 2026-03-26: Fix LOD flood bug by replacing pyramid downsample priority-max merge with dominant-terrain voting and a water-safe tie break, and add `F3` runtime debug HUD for coords + zoom visibility.
