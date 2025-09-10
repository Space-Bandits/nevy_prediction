use bevy::prelude::*;
use nevy_prediction::common::prelude::*;
use serde::{Deserialize, Serialize};

pub fn build(app: &mut App) {}

#[derive(Component, Default, Clone, Serialize, Deserialize)]
#[require(Transform)]
pub struct PlayerInput {
    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SpawnPlayer {
    pub entity: SimulationEntity,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SetPlayerPosition {
    pub entity: SimulationEntity,
    pub position: Vec3,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SetPlayerInput {
    pub entity: SimulationEntity,
    pub player: PlayerInput,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DespawnPlauer {
    pub entity: SimulationEntity,
}
