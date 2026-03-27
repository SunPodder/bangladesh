# Project: Bangladesh RPG (OSM-GIS Native)
An open-source, code-only 2D RPG of Bangladesh at 1:1 scale.

## Immediate Constraints
- Use Bevy ECS patterns (Systems/Components). No OOP.
- Performance is critical. Use `Rayon` for parallel GIS tasks.
- Single binary architecture. Always consider if code belongs in `shared`.
- Always document important design decisions in `architecture.md` and `agent.md` appropriately.
- Keep the documentation compact to avoid context bloat.
