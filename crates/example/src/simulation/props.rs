//! Props are avian rigid bodies that bounce around a walled arena, to test physics with
//! [`nevy_prediction_avian`].
//!
//! Gravity is off and the props bounce without losing energy, so they keep colliding with each other and the walls.
//! Nothing but the server affects them, so any difference between a client and the server comes from physics
//! not being deterministic across worlds.

use avian3d::prelude::*;
use bevy::prelude::*;
use nevy_prediction::prelude::*;
use nevy_prediction_avian::prelude::*;
use serde::{Deserialize, Serialize};

use crate::simulation::workload::unit;

/// Half the size of the arena, the walls are just outside of it.
pub const ARENA_HALF_SIZE: f32 = 8.;
const WALL_THICKNESS: f32 = 1.;

pub fn build(app: &mut App) {
    app.add_world_update::<SpawnProps>();

    app.insert_resource(Gravity::ZERO);

    app.add_plugins(ExtractSimulationComponentPlugin::<Prop>::default());
    app.add_plugins(ExtractSimulationComponentPlugin::<Wall>::default());

    app.add_systems(
        SimulationUpdate,
        spawn_props
            .before(UpdateComponentSystems)
            .before(PredictionPhysicsSystems::ApplyCorrections),
    );
}

/// A dynamic body. Every component that affects its physics is either extracted or required by this,
/// so that a world it was extracted into simulates it the same as the world it was spawned in.
#[derive(Component, Clone, Copy, Debug, Serialize, Deserialize)]
#[require(
    RigidBody::Dynamic,
    Restitution = Restitution::PERFECTLY_ELASTIC.with_combine_rule(CoefficientCombine::Max),
    Friction = Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
    LinearDamping(0.),
    AngularDamping(0.)
)]
pub struct Prop {
    pub half_size: f32,
}

#[derive(Component, Clone, Copy, Debug, Serialize, Deserialize)]
#[require(RigidBody::Static, Restitution = Restitution::PERFECTLY_ELASTIC, Friction = Friction::ZERO)]
pub struct Wall;

#[derive(Clone, Serialize, Deserialize)]
pub struct PropState {
    pub entity: SimulationEntity,
    pub prop: Prop,
    pub position: Vec3,
    pub rotation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
}

/// Spawns the arena walls and props.
#[derive(Clone, Serialize, Deserialize)]
pub struct SpawnProps {
    /// The walls, one for each side of the arena.
    pub walls: Option<[SimulationEntity; 6]>,
    pub props: Vec<PropState>,
}

impl SpawnProps {
    /// Creates a new arena with props scattered around it, moving in random directions.
    pub fn random(mut next_entity: impl FnMut() -> SimulationEntity, count: u32, seed: u64) -> Self {
        let walls = std::array::from_fn(|_| next_entity());

        let props = (0..count as u64)
            .map(|index| {
                let seed = crate::simulation::workload::hash(seed ^ index);
                let random = |offset| unit(seed + offset) * 2. - 1.;

                let span = ARENA_HALF_SIZE - 1.;
                PropState {
                    entity: next_entity(),
                    prop: Prop {
                        half_size: 0.2 + 0.3 * unit(seed + 7),
                    },
                    position: Vec3::new(random(1), random(2), random(3)) * span,
                    rotation: Quat::from_rotation_y(random(4) * std::f32::consts::PI),
                    linear_velocity: Vec3::new(random(5), random(6), random(8)).normalize_or_zero()
                        * 4.,
                    angular_velocity: Vec3::ZERO,
                }
            })
            .collect();

        SpawnProps {
            walls: Some(walls),
            props,
        }
    }
}

fn spawn_props(mut commands: Commands, mut updates: ReadyUpdates<SpawnProps>) {
    for SpawnProps { walls, props } in updates.drain() {
        if let Some(walls) = walls {
            let offset = ARENA_HALF_SIZE + WALL_THICKNESS / 2.;
            let long = (ARENA_HALF_SIZE + WALL_THICKNESS) * 2.;

            for (index, entity) in walls.into_iter().enumerate() {
                let axis = index / 2;
                let sign = if index % 2 == 0 { 1. } else { -1. };

                let mut position = Vec3::ZERO;
                position[axis] = offset * sign;
                let mut size = Vec3::splat(long);
                size[axis] = WALL_THICKNESS;

                commands.spawn((
                    entity,
                    Wall,
                    Position(position),
                    ColliderConstructor::Cuboid {
                        x_length: size.x,
                        y_length: size.y,
                        z_length: size.z,
                    },
                ));
            }
        }

        for state in props {
            let size = state.prop.half_size * 2.;

            commands.spawn((
                state.entity,
                state.prop,
                Position(state.position),
                Rotation(state.rotation),
                LinearVelocity(state.linear_velocity),
                AngularVelocity(state.angular_velocity),
                ColliderConstructor::Cuboid {
                    x_length: size,
                    y_length: size,
                    z_length: size,
                },
            ));
        }
    }
}
