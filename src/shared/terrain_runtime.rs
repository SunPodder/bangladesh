use crate::shared::world::{
    ArchivedTerrainChunk, TerrainKind, WorldStreamReader, world_output_path,
};
use anyhow::{Result, anyhow, ensure};
use bevy::prelude::*;
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
    load_radius: i32,
    movement_speed: f32,
}

#[derive(Resource)]
struct TerrainWorldState {
    reader: WorldStreamReader,
    loaded_chunks: HashMap<(i32, i32), LoadedChunk>,
    cells_per_side: usize,
    cell_size: f32,
}

struct LoadedChunk {
    _bytes: Vec<u8>,
    entities: Vec<Entity>,
}

#[derive(Component)]
struct StreamPlayer;

impl Plugin for TerrainStreamingPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TerrainRuntimeConfig {
            region: self.region.clone(),
            load_radius: 1,
            movement_speed: 900.0,
        });

        app.add_systems(Startup, (setup_terrain_view, open_world).chain());
        app.add_systems(
            Update,
            (move_player, follow_player_camera, stream_chunks_around_player),
        );
    }
}

fn setup_terrain_view(mut commands: Commands) {
    commands.spawn(Camera2d);

    commands.spawn((
        StreamPlayer,
        Sprite::from_color(Color::srgb(1.0, 0.2, 0.2), Vec2::splat(10.0)),
        Transform::from_xyz(0.0, 0.0, 10.0),
    ));
}

fn open_world(
    mut commands: Commands,
    config: Res<TerrainRuntimeConfig>,
    mut player_query: Query<&mut Transform, With<StreamPlayer>>,
) {
    let world_path = world_output_path(&config.region);

    match WorldStreamReader::open(&world_path) {
        Ok(reader) => {
            let cell_size = reader.index.chunk_size_m / f32::from(reader.index.cells_per_side);
            let start_x =
                (reader.index.local_bounds_min_x + reader.index.local_bounds_max_x) * 0.5;
            let start_y =
                (reader.index.local_bounds_min_y + reader.index.local_bounds_max_y) * 0.5;

            if let Ok(mut player_transform) = player_query.single_mut() {
                player_transform.translation.x = start_x;
                player_transform.translation.y = start_y;
            }

            info!(
                "Loaded world index for region '{}' ({} chunks)",
                reader.index.region,
                reader.index.chunks.len()
            );

            commands.insert_resource(TerrainWorldState {
                cells_per_side: usize::from(reader.index.cells_per_side),
                cell_size,
                reader,
                loaded_chunks: HashMap::new(),
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

fn stream_chunks_around_player(
    mut commands: Commands,
    config: Res<TerrainRuntimeConfig>,
    player_query: Query<&Transform, With<StreamPlayer>>,
    state: Option<ResMut<TerrainWorldState>>,
) {
    let Some(mut state) = state else {
        return;
    };
    let Ok(player_transform) = player_query.single() else {
        return;
    };

    let chunk_size = state.reader.index.chunk_size_m;
    let player_chunk_x = (player_transform.translation.x / chunk_size).floor() as i32;
    let player_chunk_y = (player_transform.translation.y / chunk_size).floor() as i32;

    let mut desired = HashSet::new();
    for dy in -config.load_radius..=config.load_radius {
        for dx in -config.load_radius..=config.load_radius {
            let key = (player_chunk_x + dx, player_chunk_y + dy);
            if state.reader.index.chunks.contains_key(&key) {
                desired.insert(key);
            }
        }
    }

    let to_unload: Vec<(i32, i32)> = state
        .loaded_chunks
        .keys()
        .filter(|chunk_key| !desired.contains(chunk_key))
        .copied()
        .collect();

    for chunk_key in to_unload {
        if let Some(loaded_chunk) = state.loaded_chunks.remove(&chunk_key) {
            for entity in loaded_chunk.entities {
                commands.entity(entity).despawn();
            }
        }
    }

    let to_load: Vec<(i32, i32)> = desired
        .into_iter()
        .filter(|chunk_key| !state.loaded_chunks.contains_key(chunk_key))
        .collect();

    for (chunk_x, chunk_y) in to_load {
        match state.reader.load_chunk_bytes(chunk_x, chunk_y) {
            Ok(Some(bytes)) => {
                match spawn_chunk_entities(
                    &mut commands,
                    state.cell_size,
                    state.cells_per_side,
                    chunk_x,
                    chunk_y,
                    &bytes,
                ) {
                    Ok(entities) => {
                        state
                            .loaded_chunks
                            .insert((chunk_x, chunk_y), LoadedChunk {
                                _bytes: bytes,
                                entities,
                            });
                    }
                    Err(error) => {
                        error!("Failed to spawn chunk ({chunk_x}, {chunk_y}): {error}");
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                error!("Failed to load chunk ({chunk_x}, {chunk_y}): {error}");
            }
        }
    }
}

fn spawn_chunk_entities(
    commands: &mut Commands,
    cell_size: f32,
    cells_per_side: usize,
    chunk_x: i32,
    chunk_y: i32,
    chunk_bytes: &[u8],
) -> Result<Vec<Entity>> {
    let archived = access::<ArchivedTerrainChunk, RkyvError>(chunk_bytes)
        .map_err(|err| anyhow!("failed to access archived terrain chunk: {err}"))?;

    ensure!(
        archived.chunk_x == chunk_x && archived.chunk_y == chunk_y,
        "chunk metadata mismatch while loading chunk"
    );

    let expected_cells = cells_per_side * cells_per_side;
    ensure!(
        archived.cells.len() == expected_cells,
        "chunk cell count mismatch: expected {expected_cells}, got {}",
        archived.cells.len()
    );

    let mut entities = Vec::with_capacity(expected_cells);
    let chunk_size = cell_size * cells_per_side as f32;
    let chunk_origin_x = chunk_x as f32 * chunk_size;
    let chunk_origin_y = chunk_y as f32 * chunk_size;

    for (index, terrain_code) in archived.cells.iter().copied().enumerate() {
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