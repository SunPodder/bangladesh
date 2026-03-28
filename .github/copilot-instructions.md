# Project: Bangladesh RPG (OSM-GIS Native)
An open-source, code-only 2D RPG of Bangladesh at 1:1 scale.

## Immediate Constraints
- Use Bevy ECS patterns (Systems/Components). No OOP.
- Performance is critical. Use `Rayon` for parallel GIS tasks.
- Single binary architecture. Always consider if code belongs in `shared`.
- Always document important design decisions that affect the overall architecture in `architecture.md` and `agent.md` appropriately. don't add one time decisions here. this is not a journal.
- Keep the documentation compact to avoid context bloat.
