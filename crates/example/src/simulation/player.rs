use std::time::Duration;

use bevy::prelude::*;
use nevy_prediction::common::prelude::*;
use serde::{Deserialize, Serialize};

pub fn build(app: &mut App) {
    app.add_world_update::<SpawnPlayer>();
    app.add_plugins(UpdateComponentPlugin::<PlayerInput>::default());
    app.add_plugins(UpdateComponentPlugin::<PlayerState>::default());

    app.add_plugins(ExtractSimulationComponentPlugin::<Player>::default());
    app.add_plugins(ExtractSimulationComponentPlugin::<PlayerState>::default());
    app.add_plugins(ExtractSimulationComponentPlugin::<PlayerInput>::default());

    app.add_systems(
        SimulationUpdate,
        (
            spawn_players.before(UpdateComponentSystems),
            move_players.after(UpdateComponentSystems),
        ),
    );
}

#[derive(Component, Default, Clone)]
#[require(PlayerInput, PlayerState)]
pub struct Player;

#[derive(Component, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlayerInput {
    pub forward: bool,
    pub backward: bool,
    pub right: bool,
    pub left: bool,
}

#[derive(Component, Default, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    pub position: Vec2,
    pub velocity: Vec2,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SpawnPlayer {
    pub entity: SimulationEntity,
}

/// Server -> Client message to set the local player of the client.
#[derive(Serialize, Deserialize)]
pub struct SetLocalPlayer {
    pub entity: SimulationEntity,
}
/// Client -> Server message to request movement of the player.
#[derive(Serialize, Deserialize)]
pub struct RequestMovePlayer {
    pub time: Duration,
    pub input: PlayerInput,
}

fn spawn_players(mut commands: Commands, mut updates: ReadyUpdates<SpawnPlayer>) {
    for SpawnPlayer { entity } in updates.drain() {
        let player_entity = commands.spawn((entity, Player)).id();

        debug!("Spawned player {:?} {}", entity, player_entity);
    }
}

const PLAYER_SPEED: f32 = 5.0;

fn move_players(mut player_q: Query<(&mut PlayerState, &PlayerInput)>, time: Res<Time>) {
    for (mut state, input) in player_q.iter_mut() {
        let input_vector = Vec2::new(
            match (input.forward, input.backward) {
                (true, false) => 1.0,
                (false, true) => -1.0,
                _ => 0.0,
            },
            match (input.right, input.left) {
                (true, false) => 1.0,
                (false, true) => -1.0,
                _ => 0.0,
            },
        )
        .normalize_or_zero();

        state.velocity = input_vector * PLAYER_SPEED;

        let position_delta = state.velocity * time.delta_secs();
        state.position += position_delta;
    }
}
