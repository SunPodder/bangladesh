use crate::shared::world::{
    ArchivedTerrainTile, TerrainKind, WorldIndex, WorldStreamReader, world_output_path,
};
use anyhow::{Result, anyhow, ensure};
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rkyv::{access, rancor::Error as RkyvError};
use std::collections::{HashMap, HashSet};

pub struct TerrainStreamingPlugin {
    region: String,
}

impl TerrainStreamingPlugin {
    pub fn new(region: String) -> Self {
        Self { region }
    }
}

#[derive(Resource, Clone)]
struct TerrainRuntimeConfig {
    region: String,
    movement_speed: f32,
    zoom_lerp_speed: f32,
    zoom_wheel_sensitivity_steps: f32,
    lod_hysteresis_ratio: f32,
    preload_margin_tiles: i32,
    playable_view_width_m: f32,
    overview_padding_ratio: f32,
}

#[derive(Resource)]
struct TerrainWorldState {
    reader: WorldStreamReader,
    loaded_tiles: HashMap<(u8, i32, i32), LoadedTile>,
    cells_per_side: usize,
    playable_chunk_size_m: f32,
    full_map_scale: f32,
    lod_playable_scale: f32,
    current_zoom_level: u8,
    current_zoom_step: f32,
}

#[derive(Resource)]
struct ZoomController {
    max_zoom_step: u8,
    min_scale: f32,
    max_scale: f32,
    target_scale: f32,
}

#[derive(Resource, Default)]
struct DebugOverlayState {
    visible: bool,
}

struct LoadedTile {
    _bytes: Vec<u8>,
    entities: Vec<Entity>,
}

#[derive(Component)]
struct StreamPlayer;

#[derive(Component)]
struct DebugOverlayRoot;

#[derive(Component)]
struct DebugOverlayText;

impl Plugin for TerrainStreamingPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TerrainRuntimeConfig {
            region: self.region.clone(),
            movement_speed: 900.0,
            zoom_lerp_speed: 8.0,
            zoom_wheel_sensitivity_steps: 0.25,
            lod_hysteresis_ratio: 0.10,
            preload_margin_tiles: 1,
            playable_view_width_m: 96.0,
            overview_padding_ratio: 1.08,
        });
        app.insert_resource(DebugOverlayState::default());

        app.add_systems(
            Startup,
            (setup_terrain_view, setup_debug_overlay, open_world).chain(),
        );
        app.add_systems(
            Update,
            (
                move_player,
                handle_zoom_input,
                smooth_zoom_camera,
                follow_player_camera,
                update_map_lod,
                toggle_debug_overlay,
                update_debug_overlay_text,
            )
                .chain(),
        );
    }
}

fn setup_terrain_view(mut commands: Commands) {
    commands.spawn(Camera2d);

    commands.spawn((
        StreamPlayer,
        Sprite::from_color(Color::srgb(1.0, 0.2, 0.2), Vec2::new(0.9, 1.8)),
        Transform::from_xyz(0.0, 0.0, 10.0),
    ));
}

fn setup_debug_overlay(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(12.0),
                left: Val::Px(12.0),
                padding: UiRect::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.07, 0.11, 0.80)),
            Visibility::Hidden,
            DebugOverlayRoot,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("F3 Debug"),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.97, 1.0)),
                DebugOverlayText,
            ));
        });
}

fn open_world(
    mut commands: Commands,
    config: Res<TerrainRuntimeConfig>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    mut camera_query: Query<&mut Projection, With<Camera2d>>,
    mut player_query: Query<&mut Transform, With<StreamPlayer>>,
) {
    let world_path = world_output_path(&config.region);

    match WorldStreamReader::open(&world_path) {
        Ok(reader) => {
            let start_x = (reader.index.local_bounds_min_x + reader.index.local_bounds_max_x) * 0.5;
            let start_y = (reader.index.local_bounds_min_y + reader.index.local_bounds_max_y) * 0.5;

            if let Ok(mut player_transform) = player_query.single_mut() {
                player_transform.translation.x = start_x;
                player_transform.translation.y = start_y;
            }

            info!(
                "Loaded world index for region '{}' ({} tiles across zoom 0..{})",
                reader.index.region,
                reader.index.tiles.len(),
                reader.index.playable_zoom_level,
            );
            info!(
                "Playable terrain detail: {:.2}m per cell (tile {:.1}m / {} cells)",
                reader.index.chunk_size_m / f32::from(reader.index.cells_per_side),
                reader.index.chunk_size_m,
                reader.index.cells_per_side,
            );

            let window = window_query.single().ok();
            let window_width = window.map(Window::width).unwrap_or(1280.0);
            let window_height = window.map(Window::height).unwrap_or(720.0);

            let map_width =
                (reader.index.local_bounds_max_x - reader.index.local_bounds_min_x).abs();
            let map_height =
                (reader.index.local_bounds_max_y - reader.index.local_bounds_min_y).abs();

            let full_map_scale = fit_scale_for_bounds(
                window_width,
                window_height,
                map_width,
                map_height,
                config.overview_padding_ratio,
            );
            let desired_playable_scale =
                scale_for_view_width(window_width, config.playable_view_width_m);
            let max_zoom_step = zoom_steps_for_target(full_map_scale, desired_playable_scale);
            let playable_scale = scale_for_zoom_step(full_map_scale, max_zoom_step);

            let playable_zoom_level = reader.index.playable_zoom_level;
            let lod_playable_scale = scale_for_zoom_step(full_map_scale, playable_zoom_level);

            info!(
                "Zoom profile: zoom-0 fits {:.0}m x {:.0}m (scale {:.5}), playable target {:.1}m across (step {}, scale {:.5})",
                map_width,
                map_height,
                full_map_scale,
                config.playable_view_width_m,
                max_zoom_step,
                playable_scale,
            );
            info!(
                "LOD profile: terrain zoom 0..{} (max LOD reached by zoom step {})",
                playable_zoom_level, playable_zoom_level,
            );

            if let Ok(mut projection) = camera_query.single_mut() {
                if let Projection::Orthographic(ortho) = projection.as_mut() {
                    ortho.scale = playable_scale;
                }
            }

            commands.insert_resource(TerrainWorldState {
                cells_per_side: usize::from(reader.index.cells_per_side),
                playable_chunk_size_m: reader.index.chunk_size_m,
                full_map_scale,
                lod_playable_scale,
                reader,
                loaded_tiles: HashMap::new(),
                current_zoom_level: playable_zoom_level,
                current_zoom_step: f32::from(max_zoom_step),
            });

            commands.insert_resource(ZoomController {
                max_zoom_step,
                min_scale: playable_scale,
                max_scale: full_map_scale,
                target_scale: playable_scale,
            });
        }
        Err(error) => {
            error!(
                "Unable to open world file {}: {}",
                world_path.display(),
                error
            );
        }
    }
}

fn move_player(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    config: Res<TerrainRuntimeConfig>,
    mut player_query: Query<&mut Transform, With<StreamPlayer>>,
) {
    let Ok(mut transform) = player_query.single_mut() else {
        return;
    };

    let mut direction = Vec2::ZERO;
    if keyboard.pressed(KeyCode::ArrowRight) || keyboard.pressed(KeyCode::KeyD) {
        direction.x += 1.0;
    }
    if keyboard.pressed(KeyCode::ArrowLeft) || keyboard.pressed(KeyCode::KeyA) {
        direction.x -= 1.0;
    }
    if keyboard.pressed(KeyCode::ArrowUp) || keyboard.pressed(KeyCode::KeyW) {
        direction.y += 1.0;
    }
    if keyboard.pressed(KeyCode::ArrowDown) || keyboard.pressed(KeyCode::KeyS) {
        direction.y -= 1.0;
    }

    if direction == Vec2::ZERO {
        return;
    }

    let movement = direction.normalize() * config.movement_speed * time.delta_secs();
    transform.translation.x += movement.x;
    transform.translation.y += movement.y;
}

fn handle_zoom_input(
    config: Res<TerrainRuntimeConfig>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut wheel_events: MessageReader<MouseWheel>,
    zoom_controller: Option<ResMut<ZoomController>>,
) {
    let Some(mut zoom_controller) = zoom_controller else {
        return;
    };

    let mut steps = 0.0_f32;
    for event in wheel_events.read() {
        steps += event.y;
    }

    if keyboard.just_pressed(KeyCode::Equal) || keyboard.just_pressed(KeyCode::NumpadAdd) {
        steps += 1.0;
    }
    if keyboard.just_pressed(KeyCode::Minus) || keyboard.just_pressed(KeyCode::NumpadSubtract) {
        steps -= 1.0;
    }

    if steps.abs() < f32::EPSILON {
        return;
    }

    let zoom_factor = 2.0_f32.powf(-steps * config.zoom_wheel_sensitivity_steps);
    zoom_controller.target_scale = (zoom_controller.target_scale * zoom_factor)
        .clamp(zoom_controller.min_scale, zoom_controller.max_scale);
}

fn smooth_zoom_camera(
    time: Res<Time>,
    config: Res<TerrainRuntimeConfig>,
    zoom_controller: Option<Res<ZoomController>>,
    mut camera_query: Query<&mut Projection, With<Camera2d>>,
) {
    let Some(zoom_controller) = zoom_controller else {
        return;
    };

    let Ok(mut projection) = camera_query.single_mut() else {
        return;
    };

    let Projection::Orthographic(ortho) = projection.as_mut() else {
        return;
    };

    let alpha = 1.0 - (-config.zoom_lerp_speed * time.delta_secs()).exp();
    let next_scale = ortho.scale + (zoom_controller.target_scale - ortho.scale) * alpha;

    if (zoom_controller.target_scale - next_scale).abs() < 0.001 {
        ortho.scale = zoom_controller.target_scale;
    } else {
        ortho.scale = next_scale;
    }
}

fn follow_player_camera(
    player_query: Query<&Transform, With<StreamPlayer>>,
    mut camera_query: Query<&mut Transform, (With<Camera2d>, Without<StreamPlayer>)>,
) {
    let Ok(player_transform) = player_query.single() else {
        return;
    };

    let Ok(mut camera_transform) = camera_query.single_mut() else {
        return;
    };

    camera_transform.translation.x = player_transform.translation.x;
    camera_transform.translation.y = player_transform.translation.y;
}

fn toggle_debug_overlay(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut overlay_state: ResMut<DebugOverlayState>,
    mut root_query: Query<&mut Visibility, With<DebugOverlayRoot>>,
) {
    if !keyboard.just_pressed(KeyCode::F3) {
        return;
    }

    overlay_state.visible = !overlay_state.visible;
    let next_visibility = if overlay_state.visible {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };

    for mut visibility in &mut root_query {
        *visibility = next_visibility;
    }
}

fn update_debug_overlay_text(
    overlay_state: Res<DebugOverlayState>,
    state: Option<Res<TerrainWorldState>>,
    zoom_controller: Option<Res<ZoomController>>,
    player_query: Query<&Transform, With<StreamPlayer>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    mut text_query: Query<&mut Text, With<DebugOverlayText>>,
) {
    if !overlay_state.visible {
        return;
    }

    let Ok(mut text) = text_query.single_mut() else {
        return;
    };

    let player_coords = player_query
        .single()
        .ok()
        .map(|transform| {
            (
                transform.translation.x,
                transform.translation.y,
                transform.translation.z,
            )
        })
        .unwrap_or((0.0, 0.0, 0.0));

    let cursor_coords = (|| {
        let (camera, camera_transform) = camera_query.single().ok()?;
        let window = window_query.single().ok()?;
        let cursor_screen = window.cursor_position()?;
        camera
            .viewport_to_world_2d(camera_transform, cursor_screen)
            .ok()
    })();

    let zoom_level_text = state
        .as_ref()
        .map(|world_state| world_state.current_zoom_level.to_string())
        .unwrap_or_else(|| "N/A".to_string());

    let zoom_step_text = match (state.as_ref(), zoom_controller.as_ref()) {
        (Some(world_state), Some(controller)) => {
            format!(
                "{:.2}/{}",
                world_state.current_zoom_step, controller.max_zoom_step
            )
        }
        _ => "N/A".to_string(),
    };

    let cursor_text = cursor_coords
        .map(|coords| format!("{:.1}, {:.1}", coords.x, coords.y))
        .unwrap_or_else(|| "N/A".to_string());

    text.0 = format!(
        "F3 Debug\nPlayer: ({:.1}, {:.1}, {:.1})\nCursor: ({})\nLOD Zoom: {}\nCamera Zoom Step: {}",
        player_coords.0,
        player_coords.1,
        player_coords.2,
        cursor_text,
        zoom_level_text,
        zoom_step_text,
    );
}

fn update_map_lod(
    mut commands: Commands,
    config: Res<TerrainRuntimeConfig>,
    camera_query: Query<(&Transform, &Projection), With<Camera2d>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    state: Option<ResMut<TerrainWorldState>>,
) {
    let Some(mut state) = state else {
        return;
    };

    let Ok((camera_transform, camera_projection)) = camera_query.single() else {
        return;
    };

    let Projection::Orthographic(ortho_projection) = camera_projection else {
        return;
    };

    let camera_scale = ortho_projection.scale;
    let desired_zoom = select_lod_with_hysteresis(
        state.reader.index.playable_zoom_level,
        state.lod_playable_scale,
        camera_scale,
        state.current_zoom_level,
        config.lod_hysteresis_ratio,
    );
    state.current_zoom_level = desired_zoom;
    state.current_zoom_step = zoom_progress_for_scale(state.full_map_scale, camera_scale);

    let Ok(window) = window_query.single() else {
        return;
    };

    let half_width = 0.5 * window.width() * camera_scale;
    let half_height = 0.5 * window.height() * camera_scale;

    let min_x = camera_transform.translation.x - half_width;
    let max_x = camera_transform.translation.x + half_width;
    let min_y = camera_transform.translation.y - half_height;
    let max_y = camera_transform.translation.y + half_height;

    let min_tile_x =
        world_to_tile_x(&state.reader.index, desired_zoom, min_x) - config.preload_margin_tiles;
    let max_tile_x =
        world_to_tile_x(&state.reader.index, desired_zoom, max_x) + config.preload_margin_tiles;
    let min_tile_y =
        world_to_tile_y(&state.reader.index, desired_zoom, min_y) - config.preload_margin_tiles;
    let max_tile_y =
        world_to_tile_y(&state.reader.index, desired_zoom, max_y) + config.preload_margin_tiles;

    let mut desired_tiles = HashSet::new();
    for tile_y in min_tile_y..=max_tile_y {
        for tile_x in min_tile_x..=max_tile_x {
            let key = (desired_zoom, tile_x, tile_y);
            if state.reader.index.tiles.contains_key(&key) {
                desired_tiles.insert(key);
            }
        }
    }

    let to_unload: Vec<(u8, i32, i32)> = state
        .loaded_tiles
        .keys()
        .filter(|tile_key| !desired_tiles.contains(tile_key))
        .copied()
        .collect();

    for tile_key in to_unload {
        if let Some(loaded_tile) = state.loaded_tiles.remove(&tile_key) {
            for entity in loaded_tile.entities {
                commands.entity(entity).despawn();
            }
        }
    }

    let to_load: Vec<(u8, i32, i32)> = desired_tiles
        .into_iter()
        .filter(|tile_key| !state.loaded_tiles.contains_key(tile_key))
        .collect();

    let playable_zoom = state.reader.index.playable_zoom_level;
    let playable_tile_offset_x = state.reader.index.playable_tile_offset_x;
    let playable_tile_offset_y = state.reader.index.playable_tile_offset_y;

    for (zoom, tile_x, tile_y) in to_load {
        match state.reader.load_tile_bytes(zoom, tile_x, tile_y) {
            Ok(Some(bytes)) => {
                match spawn_tile_entities(
                    &mut commands,
                    state.playable_chunk_size_m,
                    state.cells_per_side,
                    playable_zoom,
                    playable_tile_offset_x,
                    playable_tile_offset_y,
                    zoom,
                    tile_x,
                    tile_y,
                    &bytes,
                ) {
                    Ok(entities) => {
                        state.loaded_tiles.insert(
                            (zoom, tile_x, tile_y),
                            LoadedTile {
                                _bytes: bytes,
                                entities,
                            },
                        );
                    }
                    Err(error) => {
                        error!("Failed to spawn tile ({zoom}, {tile_x}, {tile_y}): {error}");
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                error!("Failed to load tile ({zoom}, {tile_x}, {tile_y}): {error}");
            }
        }
    }
}

fn spawn_tile_entities(
    commands: &mut Commands,
    playable_chunk_size_m: f32,
    cells_per_side: usize,
    playable_zoom_level: u8,
    playable_tile_offset_x: i32,
    playable_tile_offset_y: i32,
    zoom: u8,
    tile_x: i32,
    tile_y: i32,
    tile_bytes: &[u8],
) -> Result<Vec<Entity>> {
    let archived = access::<ArchivedTerrainTile, RkyvError>(tile_bytes)
        .map_err(|err| anyhow!("failed to access archived terrain tile: {err}"))?;

    let archived_tile_x: i32 = archived.tile_x.into();
    let archived_tile_y: i32 = archived.tile_y.into();
    ensure!(
        u8::from(archived.zoom) == zoom && archived_tile_x == tile_x && archived_tile_y == tile_y,
        "tile metadata mismatch while loading tile"
    );

    let expected_cells = cells_per_side * cells_per_side;
    ensure!(
        archived.cells.len() == expected_cells,
        "tile cell count mismatch: expected {expected_cells}, got {}",
        archived.cells.len()
    );

    let mut entities = Vec::with_capacity(expected_cells);
    let zoom_factor = 1_i32 << u32::from(playable_zoom_level - zoom);
    let tile_size = playable_chunk_size_m * zoom_factor as f32;
    let cell_size = tile_size / cells_per_side as f32;

    let playable_tile_x = tile_x * zoom_factor;
    let playable_tile_y = tile_y * zoom_factor;

    let chunk_origin_x = (playable_tile_x - playable_tile_offset_x) as f32 * playable_chunk_size_m;
    let chunk_origin_y = (playable_tile_y - playable_tile_offset_y) as f32 * playable_chunk_size_m;

    for (index, terrain_code_le) in archived.cells.iter().enumerate() {
        let terrain_code: u8 = (*terrain_code_le).into();
        let ix = (index % cells_per_side) as f32;
        let iy = (index / cells_per_side) as f32;

        let x = chunk_origin_x + (ix + 0.5) * cell_size;
        let y = chunk_origin_y + (iy + 0.5) * cell_size;
        let color = terrain_color(TerrainKind::from_code(terrain_code));

        let entity = commands
            .spawn((
                Sprite::from_color(color, Vec2::ONE),
                Transform {
                    translation: Vec3::new(x, y, 0.0),
                    scale: Vec3::new(cell_size, cell_size, 1.0),
                    ..default()
                },
            ))
            .id();

        entities.push(entity);
    }

    Ok(entities)
}

fn terrain_color(terrain: TerrainKind) -> Color {
    match terrain {
        TerrainKind::Unknown => Color::srgb(0.12, 0.12, 0.14),
        TerrainKind::Water => Color::srgb(0.10, 0.32, 0.68),
        TerrainKind::Grass => Color::srgb(0.27, 0.53, 0.23),
        TerrainKind::Forest => Color::srgb(0.07, 0.34, 0.16),
        TerrainKind::Urban => Color::srgb(0.47, 0.43, 0.40),
        TerrainKind::Farmland => Color::srgb(0.60, 0.54, 0.22),
        TerrainKind::Sand => Color::srgb(0.80, 0.75, 0.52),
    }
}

fn scale_for_view_width(window_width: f32, view_width_m: f32) -> f32 {
    let safe_width = window_width.max(1.0);
    (view_width_m / safe_width).max(0.0001)
}

fn fit_scale_for_bounds(
    window_width: f32,
    window_height: f32,
    bounds_width: f32,
    bounds_height: f32,
    padding_ratio: f32,
) -> f32 {
    let safe_width = window_width.max(1.0);
    let safe_height = window_height.max(1.0);
    let fit_x = bounds_width.max(1.0) / safe_width;
    let fit_y = bounds_height.max(1.0) / safe_height;
    (fit_x.max(fit_y) * padding_ratio.max(1.0)).max(0.0001)
}

fn zoom_steps_for_target(full_map_scale: f32, target_scale: f32) -> u8 {
    let full = full_map_scale.max(0.0001);
    let target = target_scale.max(0.0001);
    if full <= target {
        return 0;
    }

    (full / target).log2().ceil().max(0.0) as u8
}

fn scale_for_zoom_step(full_map_scale: f32, zoom_step: u8) -> f32 {
    full_map_scale / 2.0_f32.powi(i32::from(zoom_step))
}

fn zoom_progress_for_scale(full_map_scale: f32, scale: f32) -> f32 {
    let safe_scale = scale.max(0.0001);
    if safe_scale >= full_map_scale {
        return 0.0;
    }

    (full_map_scale / safe_scale).log2().max(0.0)
}

fn scale_for_lod(playable_zoom_level: u8, zoom_level: u8, playable_lod_scale: f32) -> f32 {
    let steps_out = i32::from(playable_zoom_level) - i32::from(zoom_level);
    playable_lod_scale * 2.0_f32.powi(steps_out.max(0))
}

fn select_lod_with_hysteresis(
    playable_zoom_level: u8,
    playable_lod_scale: f32,
    scale: f32,
    current_zoom_level: u8,
    hysteresis_ratio: f32,
) -> u8 {
    let mut zoom = i32::from(current_zoom_level).clamp(0, i32::from(playable_zoom_level));
    let hysteresis = (1.0 + hysteresis_ratio.max(0.0)).max(1.0);
    let sqrt_two = std::f32::consts::SQRT_2;

    for _ in 0..=playable_zoom_level {
        let level_scale = scale_for_lod(playable_zoom_level, zoom as u8, playable_lod_scale);
        let zoom_out_threshold = level_scale * sqrt_two * hysteresis;
        let zoom_in_threshold = (level_scale / sqrt_two) / hysteresis;

        if zoom > 0 && scale > zoom_out_threshold {
            zoom -= 1;
            continue;
        }

        if zoom < i32::from(playable_zoom_level) && scale < zoom_in_threshold {
            zoom += 1;
            continue;
        }

        break;
    }

    zoom as u8
}

fn world_to_tile_x(index: &WorldIndex, zoom_level: u8, world_x: f32) -> i32 {
    let playable_x = (world_x / index.chunk_size_m).floor() as i32 + index.playable_tile_offset_x;
    let factor = 1_i32 << u32::from(index.playable_zoom_level - zoom_level);
    playable_x.div_euclid(factor)
}

fn world_to_tile_y(index: &WorldIndex, zoom_level: u8, world_y: f32) -> i32 {
    let playable_y = (world_y / index.chunk_size_m).floor() as i32 + index.playable_tile_offset_y;
    let factor = 1_i32 << u32::from(index.playable_zoom_level - zoom_level);
    playable_y.div_euclid(factor)
}
