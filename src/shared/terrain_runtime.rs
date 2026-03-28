use crate::shared::world::{
    ArchivedRoadClass, ArchivedTerrainKind, ArchivedTileData, MapIndex, MappedTile,
    QUANTIZATION_SCALE, map_index_path, map_tile_path, read_map_index,
};
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
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
    preload_margin_tiles: i32,
    playable_view_width_m: f32,
    overview_padding_ratio: f32,
}

#[derive(Resource)]
struct TerrainWorldState {
    index: MapIndex,
    available_tiles: HashSet<u32>,
    loaded_tiles: HashMap<u32, LoadedTile>,
    current_lod: u8,
}

#[derive(Resource)]
struct ZoomController {
    min_scale: f32,
    max_scale: f32,
    target_scale: f32,
}

#[derive(Resource, Default)]
struct DebugOverlayState {
    visible: bool,
}

struct LoadedTile {
    mapped: MappedTile,
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
            preload_margin_tiles: 1,
            playable_view_width_m: 800.0,
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
                update_loaded_tiles,
                draw_loaded_tiles,
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
        Sprite::from_color(Color::srgb(0.92, 0.16, 0.16), Vec2::new(6.0, 10.0)),
        Transform::from_xyz(0.0, 0.0, 20.0),
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
    mut projection_query: Query<&mut Projection, With<Camera2d>>,
    mut player_query: Query<&mut Transform, With<StreamPlayer>>,
) {
    let index_path = map_index_path(&config.region);
    match read_map_index(&index_path) {
        Ok(index) => {
            let available_tiles = index.tiles.iter().map(|tile| tile.id).collect();
            let bounds = &index.world_bounds_mercator;
            let start_x = ((bounds.min_x + bounds.max_x) * 0.5) as f32;
            let start_y = ((bounds.min_y + bounds.max_y) * 0.5) as f32;

            if let Ok(mut player_transform) = player_query.single_mut() {
                player_transform.translation.x = start_x;
                player_transform.translation.y = start_y;
            }

            let window = window_query.single().ok();
            let window_width = window.map(Window::width).unwrap_or(1280.0);
            let window_height = window.map(Window::height).unwrap_or(720.0);
            let map_width = (bounds.max_x - bounds.min_x) as f32;
            let map_height = (bounds.max_y - bounds.min_y) as f32;
            let full_map_scale = fit_scale_for_bounds(
                window_width,
                window_height,
                map_width,
                map_height,
                config.overview_padding_ratio,
            );
            let playable_scale = scale_for_view_width(window_width, config.playable_view_width_m);
            let screen_span = window_width.max(window_height);
            let min_scale = index
                .lod_viewing_distances_m
                .first()
                .map(|distance| scale_for_view_radius(screen_span, *distance * 0.5))
                .unwrap_or(playable_scale)
                .min(playable_scale);
            let max_scale = index
                .lod_viewing_distances_m
                .last()
                .map(|distance| scale_for_view_radius(screen_span, *distance))
                .unwrap_or(full_map_scale)
                .max(full_map_scale)
                .max(min_scale);
            let initial_scale = playable_scale.clamp(min_scale, max_scale);

            if let Ok(mut projection) = projection_query.single_mut() {
                if let Projection::Orthographic(ortho) = projection.as_mut() {
                    ortho.scale = initial_scale;
                }
            }

            let current_lod =
                select_lod_for_scale(&index, initial_scale, window_width, window_height);

            commands.insert_resource(TerrainWorldState {
                index,
                available_tiles,
                loaded_tiles: HashMap::new(),
                current_lod,
            });
            commands.insert_resource(ZoomController {
                min_scale,
                max_scale,
                target_scale: initial_scale,
            });
        }
        Err(error) => {
            error!(
                "Unable to open map index {}: {}",
                index_path.display(),
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

    if direction != Vec2::ZERO {
        let delta = direction.normalize() * config.movement_speed * time.delta_secs();
        transform.translation.x += delta.x;
        transform.translation.y += delta.y;
    }
}

fn handle_zoom_input(
    mut mouse_wheel_events: MessageReader<MouseWheel>,
    config: Res<TerrainRuntimeConfig>,
    zoom_controller: Option<ResMut<ZoomController>>,
) {
    let Some(mut zoom_controller) = zoom_controller else {
        return;
    };

    let scroll_delta = mouse_wheel_events.read().map(|event| event.y).sum::<f32>();
    if scroll_delta.abs() <= f32::EPSILON {
        return;
    }

    let zoom_factor = 2.0_f32.powf(-scroll_delta * config.zoom_wheel_sensitivity_steps);
    zoom_controller.target_scale = (zoom_controller.target_scale * zoom_factor)
        .clamp(zoom_controller.min_scale, zoom_controller.max_scale);
}

fn smooth_zoom_camera(
    time: Res<Time>,
    config: Res<TerrainRuntimeConfig>,
    zoom_controller: Option<Res<ZoomController>>,
    mut projection_query: Query<&mut Projection, With<Camera2d>>,
) {
    let Some(zoom_controller) = zoom_controller else {
        return;
    };

    let Ok(mut projection) = projection_query.single_mut() else {
        return;
    };
    let Projection::Orthographic(ortho) = projection.as_mut() else {
        return;
    };

    let t = 1.0 - (-config.zoom_lerp_speed * time.delta_secs()).exp();
    ortho.scale = ortho.scale.lerp(zoom_controller.target_scale, t);
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

fn update_loaded_tiles(
    config: Res<TerrainRuntimeConfig>,
    state: Option<ResMut<TerrainWorldState>>,
    projection_query: Query<&Projection, With<Camera2d>>,
    camera_query: Query<&Transform, (With<Camera2d>, Without<StreamPlayer>)>,
    window_query: Query<&Window, With<PrimaryWindow>>,
) {
    let Some(mut state) = state else {
        return;
    };
    let Ok(projection) = projection_query.single() else {
        return;
    };
    let Ok(camera_transform) = camera_query.single() else {
        return;
    };
    let Ok(window) = window_query.single() else {
        return;
    };

    let scale = match projection {
        Projection::Orthographic(ortho) => ortho.scale,
        _ => return,
    };

    state.current_lod = select_lod_for_scale(&state.index, scale, window.width(), window.height());

    let visible = visible_tile_ids(
        &state.index,
        camera_transform.translation.truncate(),
        scale,
        window.width(),
        window.height(),
        config.preload_margin_tiles,
    );

    let desired: HashSet<u32> = visible.into_iter().collect();
    state
        .loaded_tiles
        .retain(|tile_id, _| desired.contains(tile_id));

    for tile_id in desired {
        if state.loaded_tiles.contains_key(&tile_id) {
            continue;
        }
        if !state.available_tiles.contains(&tile_id) {
            continue;
        }

        let tile_path = map_tile_path(&state.index.region, tile_id);
        match MappedTile::open(&tile_path) {
            Ok(mapped) => {
                state.loaded_tiles.insert(tile_id, LoadedTile { mapped });
            }
            Err(error) => {
                error!(
                    "Failed to load tile {} from {}: {}",
                    tile_id,
                    tile_path.display(),
                    error
                );
            }
        }
    }
}

fn draw_loaded_tiles(state: Option<Res<TerrainWorldState>>, mut gizmos: Gizmos) {
    let Some(state) = state else {
        return;
    };

    for loaded_tile in state.loaded_tiles.values() {
        let Ok(tile) = loaded_tile.mapped.archived() else {
            continue;
        };
        let lod = tile
            .lods
            .get(usize::from(state.current_lod))
            .or_else(|| tile.lods.last())
            .unwrap();

        for area in lod.areas.iter() {
            let color = terrain_color(&area.kind);
            for ring in area.rings.iter() {
                let points = ring
                    .iter()
                    .map(|point| tile_to_world(tile, point.x.into(), point.y.into()))
                    .collect::<Vec<_>>();
                if points.len() >= 2 {
                    for window in points.windows(2) {
                        gizmos.line_2d(window[0], window[1], color);
                    }
                }
            }
        }

        for building in lod.buildings.iter() {
            let points = building
                .footprint
                .iter()
                .map(|point| tile_to_world(tile, point.x.into(), point.y.into()))
                .collect::<Vec<_>>();
            if points.len() >= 2 {
                for window in points.windows(2) {
                    gizmos.line_2d(window[0], window[1], Color::srgb(0.69, 0.63, 0.57));
                }
            }
        }

        for road in lod.roads.iter() {
            let color = match road.class {
                ArchivedRoadClass::Motorway => Color::srgb(0.75, 0.29, 0.18),
                ArchivedRoadClass::Primary => Color::srgb(0.82, 0.47, 0.18),
                ArchivedRoadClass::Secondary => Color::srgb(0.90, 0.73, 0.35),
                ArchivedRoadClass::Local => Color::srgb(0.18, 0.18, 0.19),
                ArchivedRoadClass::Service => Color::srgb(0.30, 0.30, 0.33),
                ArchivedRoadClass::Track => Color::srgb(0.47, 0.42, 0.28),
            };
            let points = road
                .points
                .iter()
                .map(|point| tile_to_world(tile, point.x.into(), point.y.into()))
                .collect::<Vec<_>>();
            for window in points.windows(2) {
                gizmos.line_2d(window[0], window[1], color);
            }
        }

        for poi in lod.pois.iter() {
            let point = tile_to_world(tile, poi.point.x.into(), poi.point.y.into());
            gizmos.circle_2d(point, 18.0, Color::srgb(0.92, 0.16, 0.16));
        }
    }
}

fn toggle_debug_overlay(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut overlay_state: ResMut<DebugOverlayState>,
    mut overlay_query: Query<&mut Visibility, With<DebugOverlayRoot>>,
) {
    if !keyboard.just_pressed(KeyCode::F3) {
        return;
    }

    overlay_state.visible = !overlay_state.visible;
    if let Ok(mut visibility) = overlay_query.single_mut() {
        *visibility = if overlay_state.visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

fn update_debug_overlay_text(
    overlay_state: Res<DebugOverlayState>,
    state: Option<Res<TerrainWorldState>>,
    player_query: Query<&Transform, With<StreamPlayer>>,
    projection_query: Query<&Projection, With<Camera2d>>,
    mut text_query: Query<&mut Text, With<DebugOverlayText>>,
) {
    if !overlay_state.visible {
        return;
    }

    let Some(state) = state else {
        return;
    };
    let Ok(player_transform) = player_query.single() else {
        return;
    };
    let Ok(projection) = projection_query.single() else {
        return;
    };
    let Ok(mut text) = text_query.single_mut() else {
        return;
    };

    let scale = match projection {
        Projection::Orthographic(ortho) => ortho.scale,
        _ => 1.0,
    };

    text.0 = format!(
        "Region: {}\nPlayer: {:.0}, {:.0}\nScale: {:.4}\nLOD: {}\nLoaded tiles: {}",
        state.index.region,
        player_transform.translation.x,
        player_transform.translation.y,
        scale,
        state.current_lod,
        state.loaded_tiles.len(),
    );
}

fn visible_tile_ids(
    index: &MapIndex,
    center: Vec2,
    scale: f32,
    window_width: f32,
    window_height: f32,
    preload_margin_tiles: i32,
) -> Vec<u32> {
    let half_width = window_width.max(1.0) * scale * 0.5;
    let half_height = window_height.max(1.0) * scale * 0.5;
    let min_x = center.x - half_width;
    let max_x = center.x + half_width;
    let min_y = center.y - half_height;
    let max_y = center.y + half_height;
    let tile_size = index.tile_grid.tile_size_m as f32;

    let min_tile_x = (((min_x - index.tile_grid.origin_x_m as f32) / tile_size).floor() as i32)
        - preload_margin_tiles;
    let max_tile_x = (((max_x - index.tile_grid.origin_x_m as f32) / tile_size).floor() as i32)
        + preload_margin_tiles;
    let min_tile_y = (((min_y - index.tile_grid.origin_y_m as f32) / tile_size).floor() as i32)
        - preload_margin_tiles;
    let max_tile_y = (((max_y - index.tile_grid.origin_y_m as f32) / tile_size).floor() as i32)
        + preload_margin_tiles;

    let mut ids = Vec::new();
    for tile_y in min_tile_y.max(0)..=max_tile_y.min(index.tile_grid.rows as i32 - 1) {
        for tile_x in min_tile_x.max(0)..=max_tile_x.min(index.tile_grid.cols as i32 - 1) {
            ids.push(tile_y as u32 * index.tile_grid.cols + tile_x as u32);
        }
    }

    ids
}

fn tile_to_world(tile: &ArchivedTileData, x: i32, y: i32) -> Vec2 {
    let scale = QUANTIZATION_SCALE as f64;
    Vec2::new(
        (f64::from(tile.origin_x_m) + f64::from(x) / scale) as f32,
        (f64::from(tile.origin_y_m) + f64::from(y) / scale) as f32,
    )
}

fn terrain_color(terrain: &ArchivedTerrainKind) -> Color {
    match terrain {
        ArchivedTerrainKind::Unknown => Color::srgb(0.18, 0.18, 0.20),
        ArchivedTerrainKind::Water => Color::srgb(0.10, 0.32, 0.68),
        ArchivedTerrainKind::Grass => Color::srgb(0.27, 0.53, 0.23),
        ArchivedTerrainKind::Forest => Color::srgb(0.07, 0.34, 0.16),
        ArchivedTerrainKind::Urban => Color::srgb(0.47, 0.43, 0.40),
        ArchivedTerrainKind::Farmland => Color::srgb(0.60, 0.54, 0.22),
        ArchivedTerrainKind::Sand => Color::srgb(0.80, 0.75, 0.52),
        ArchivedTerrainKind::Road => Color::srgb(0.16, 0.16, 0.17),
    }
}

fn scale_for_view_width(window_width: f32, view_width_m: f32) -> f32 {
    (view_width_m / window_width.max(1.0)).max(0.0001)
}

fn scale_for_view_radius(screen_span: f32, view_radius_m: f32) -> f32 {
    ((view_radius_m * 2.0) / screen_span.max(1.0)).max(0.0001)
}

fn fit_scale_for_bounds(
    window_width: f32,
    window_height: f32,
    bounds_width: f32,
    bounds_height: f32,
    padding_ratio: f32,
) -> f32 {
    let fit_x = bounds_width.max(1.0) / window_width.max(1.0);
    let fit_y = bounds_height.max(1.0) / window_height.max(1.0);
    (fit_x.max(fit_y) * padding_ratio.max(1.0)).max(0.0001)
}

fn select_lod_for_scale(index: &MapIndex, scale: f32, window_width: f32, window_height: f32) -> u8 {
    let view_radius = (window_width.max(window_height) * scale) * 0.5;
    index
        .lod_viewing_distances_m
        .iter()
        .position(|distance| view_radius <= *distance)
        .unwrap_or(index.lod_viewing_distances_m.len().saturating_sub(1)) as u8
}
