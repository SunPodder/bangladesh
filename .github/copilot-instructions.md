# Project: Bangladesh RPG (OSM-GIS Native)
An open-source, code-only 2D RPG of Bangladesh at 1:5 scale.

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

## Immediate Constraints
- Use Bevy ECS patterns (Systems/Components). No OOP.
- Performance is critical. Use `Rayon` for parallel GIS tasks.
- Single binary architecture. Always consider if code belongs in `shared`.
