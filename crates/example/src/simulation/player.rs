use bevy::prelude::*;
use nevy_prediction::common::prelude::*;
use serde::{Deserialize, Serialize};

pub fn build(app: &mut App) {
    app.add_world_update::<SpawnPlayer>();
    app.add_plugins(UpdateComponentPlugin::<PlayerInput>::default());
    app.add_plugins(UpdateComponentPlugin::<PlayerState>::default());

    app.add_systems(SimulationUpdate, spawn_players);
}

#[derive(Component, Default, Clone, Serialize, Deserialize)]
pub struct PlayerInput {
    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
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

fn spawn_players(mut commands: Commands, mut updates: ReadyUpdates<SpawnPlayer>) {
    for SpawnPlayer { entity } in updates.drain() {
        commands.spawn((entity, PlayerInput::default(), PlayerState::default()));
    }
}
