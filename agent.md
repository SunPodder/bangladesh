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
