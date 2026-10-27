//! Physics in the app's world, which never steps the simulation.
//!
//! After every extraction into an app's world:
//!
//! - The parts of avian that keep [`SpatialQuery`] and collider hierarchies current run in [`PresentationPhysics`].
//!   They use the latest extracted state, which is up to a tick ahead of what the client presents.
//! - [`Transform`] is set from [`Position`] and [`Rotation`] on bodies that don't have a parent.
//!
//! On the client every body without a parent gets an [`InterpolatedBody`],
//! and in [`PhysicsPresentationSystems`] its [`Transform`] is interpolated between the latest two ticks
//! with [`Time<PredictionTime>`].
//! Corrections from finished predictions are blended in over one tick the same way.
//!
//! [`SpatialQuery`]: avian3d::prelude::SpatialQuery

use std::time::Duration;

use avian3d::{
    dynamics::solver::{
        islands::{IslandPlugin, IslandSleepingPlugin},
        joint_graph::JointGraphPlugin,
        solver_body::SolverBodyPlugin,
    },
    prelude::*,
};
use bevy::{ecs::schedule::ScheduleLabel, prelude::*, transform::TransformSystems};
use nevy_prediction::prelude::*;

/// The schedule that avian runs in in app worlds, after every extraction.
///
/// It doesn't step bodies, it only has the plugins needed for spatial queries and collider hierarchies.
/// Collision detection doesn't run, so there are no contacts or collision events in app worlds.
#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PresentationPhysics;

/// System sets for physics in app worlds.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PhysicsPresentationSystems {
    /// In the [`ExtractSimulation`] schedule of app worlds, after simulation entities are extracted,
    /// where [`PresentationPhysics`] runs and bodies' [`Transform`]s are written.
    ///
    /// Build colliders that aren't extracted, such as children of simulation entities that each world spawns itself,
    /// in [`ExtractSimulation`] before this set so spatial queries can see them.
    UpdatePhysics,
    /// In the client app's [`PostUpdate`] before transform propagation,
    /// where [`InterpolatedBody`]s are written to [`Transform`].
    Interpolate,
    /// After [`Self::Interpolate`], where [`FollowSimulationBody`] entities are moved to their bodies.
    Follow,
}

pub(crate) fn build(app: &mut App, instance: SimulationInstance, length_unit: f32) {
    let plugins = PhysicsPlugins::new(PresentationPhysics)
        .with_length_unit(length_unit)
        .build()
        .disable::<ForcePlugin>()
        // computed mass properties are extracted
        .disable::<MassPropertyPlugin>()
        .disable::<NarrowPhasePlugin<Collider>>()
        .disable::<SolverBodyPlugin>()
        .disable::<SolverSchedulePlugin>()
        .disable::<IntegratorPlugin>()
        .disable::<SolverPlugin>()
        .disable::<CcdPlugin>()
        .disable::<IslandSleepingPlugin>()
        .disable::<IslandPlugin>()
        .disable::<JointGraphPlugin<FixedJoint>>()
        .disable::<JointGraphPlugin<RevoluteJoint>>()
        .disable::<JointGraphPlugin<PrismaticJoint>>()
        .disable::<JointGraphPlugin<DistanceJoint>>()
        .disable::<JointGraphPlugin<SphericalJoint>>()
        .disable::<XpbdSolverPlugin>()
        .disable::<BvhBroadPhasePlugin>()
        .disable::<JointPlugin>()
        .disable::<PhysicsTransformPlugin>()
        .disable::<PhysicsInterpolationPlugin>()
        .disable::<ColliderCachePlugin>();

    app.add_plugins(plugins);

    // Normally added by the narrow phase, but collider AABBs need it.
    app.init_resource::<NarrowPhaseConfig>();

    // Normally configured by the solver, the collider trees finish optimizing in this set.
    app.configure_sets(
        PhysicsSchedule,
        SolverSystems::Finalize.in_set(PhysicsStepSystems::Solver),
    );

    app.configure_sets(
        ExtractSimulation,
        PhysicsPresentationSystems::UpdatePhysics
            .after(ExtractSimulationSystems::DespawnSimulationEntities),
    );

    app.add_systems(
        ExtractSimulation,
        (run_presentation_physics, write_body_transforms)
            .chain()
            .in_set(PhysicsPresentationSystems::UpdatePhysics),
    );

    if let SimulationInstance::ClientApp = instance {
        app.configure_sets(
            PostUpdate,
            (
                PhysicsPresentationSystems::Interpolate,
                PhysicsPresentationSystems::Follow,
            )
                .chain()
                .before(TransformSystems::Propagate),
        );

        app.add_systems(
            ExtractSimulation,
            record_interpolation_samples.after(PhysicsPresentationSystems::UpdatePhysics),
        );

        app.add_systems(
            PostUpdate,
            (
                interpolate_bodies.in_set(PhysicsPresentationSystems::Interpolate),
                follow_simulation_bodies.in_set(PhysicsPresentationSystems::Follow),
            ),
        );
    }
}

/// Runs [`PresentationPhysics`] without letting it change the extracted physics clock.
fn run_presentation_physics(world: &mut World) {
    let physics_time = *world.resource::<Time<Physics>>();
    let substep_time = *world.resource::<Time<Substeps>>();
    let generic_time = *world.resource::<Time>();

    // avian skips its schedule entirely on a zero delta
    let mut time = Time::<()>::default();
    time.advance_by(Duration::from_millis(1));
    *world.resource_mut::<Time>() = time;

    world.run_schedule(PresentationPhysics);

    *world.resource_mut::<Time>() = generic_time;
    *world.resource_mut::<Time<Substeps>>() = substep_time;
    *world.resource_mut::<Time<Physics>>() = physics_time;
}

fn write_body_transforms(
    mut body_q: Query<(&Position, &Rotation, &mut Transform), (With<RigidBody>, Without<ChildOf>)>,
) {
    for (position, rotation, mut transform) in &mut body_q {
        transform.translation = position.0;
        transform.rotation = rotation.0;
    }
}

/// The state of a body at a tick.
#[derive(Clone, Copy, Debug)]
pub struct BodySample {
    pub tick: SimulationTick,
    pub position: Vec3,
    pub rotation: Quat,
}

/// Interpolates a body between the latest two ticks that were extracted into the client app.
///
/// Inserted automatically on every [`RigidBody`] without a parent in the client app.
#[derive(Component, Clone, Copy, Debug)]
pub struct InterpolatedBody {
    pub previous: BodySample,
    pub current: BodySample,
}

impl InterpolatedBody {
    /// Samples the body at a fractional tick, clamped to the two samples.
    pub fn sample(&self, tick: f64) -> (Vec3, Quat) {
        let previous = self.previous.tick.0 as f64;
        let current = self.current.tick.0 as f64;

        // Usually one tick apart, but more when the app missed ticks.
        let s = if current > previous {
            ((tick - previous) / (current - previous)).clamp(0., 1.) as f32
        } else {
            1.
        };

        (
            self.previous.position.lerp(self.current.position, s),
            self.previous.rotation.slerp(self.current.rotation, s),
        )
    }
}

fn record_interpolation_samples(
    mut commands: Commands,
    time: Res<Time<SimulationTime>>,
    mut body_q: Query<
        (Entity, &Position, &Rotation, Option<&mut InterpolatedBody>),
        (With<RigidBody>, With<SimulationEntity>, Without<ChildOf>),
    >,
) {
    let tick = time.current_tick();

    for (entity, position, rotation, body) in &mut body_q {
        let sample = BodySample {
            tick,
            position: position.0,
            rotation: rotation.0,
        };

        match body {
            Some(mut body) => {
                // A finished prediction can correct a tick that was already extracted.
                if body.current.tick != tick {
                    body.previous = body.current;
                }
                body.current = sample;
            }
            None => {
                commands.entity(entity).insert(InterpolatedBody {
                    previous: sample,
                    current: sample,
                });
            }
        }
    }
}

fn interpolate_bodies(
    time: Res<Time<PredictionTime>>,
    mut body_q: Query<(&InterpolatedBody, &mut Transform)>,
) {
    // `fraction` is between the tick before the latest one and the latest one
    let tick = time.latest_tick().0 as f64 - 1. + time.fraction() as f64;

    for (body, mut transform) in &mut body_q {
        let (position, rotation) = body.sample(tick);
        transform.translation = position;
        transform.rotation = rotation;
    }
}

/// Moves an entity in the client app with a simulation body's interpolated [`Transform`],
/// such as a camera rig following the local player.
#[derive(Component, Clone, Copy, Debug)]
pub struct FollowSimulationBody {
    pub simulation_entity: SimulationEntity,
    pub translation: bool,
    pub rotation: bool,
}

impl FollowSimulationBody {
    pub fn new(simulation_entity: SimulationEntity) -> Self {
        FollowSimulationBody {
            simulation_entity,
            translation: true,
            rotation: true,
        }
    }

    pub fn only_translation(simulation_entity: SimulationEntity) -> Self {
        FollowSimulationBody {
            rotation: false,
            ..Self::new(simulation_entity)
        }
    }

    pub fn only_rotation(simulation_entity: SimulationEntity) -> Self {
        FollowSimulationBody {
            translation: false,
            ..Self::new(simulation_entity)
        }
    }
}

fn follow_simulation_bodies(
    map: Res<SimulationEntityMap>,
    mut follower_q: Query<(&FollowSimulationBody, &mut Transform)>,
    body_q: Query<&Transform, Without<FollowSimulationBody>>,
) {
    for (follow, mut transform) in &mut follower_q {
        let Some(body) = map.get(follow.simulation_entity).and_then(|entity| body_q.get(entity).ok()) else {
            continue;
        };

        if follow.translation {
            transform.translation = body.translation;
        }
        if follow.rotation {
            transform.rotation = body.rotation;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(previous: u32, current: u32) -> InterpolatedBody {
        InterpolatedBody {
            previous: BodySample {
                tick: SimulationTick(previous),
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
            },
            current: BodySample {
                tick: SimulationTick(current),
                position: Vec3::X,
                rotation: Quat::from_rotation_y(1.),
            },
        }
    }

    #[test]
    fn interpolates_between_samples() {
        let (position, rotation) = body(10, 11).sample(10.25);
        assert!((position.x - 0.25).abs() < 1e-5);
        assert!(rotation.angle_between(Quat::from_rotation_y(0.25)) < 1e-4);
    }

    #[test]
    fn clamps_to_samples() {
        assert_eq!(body(10, 11).sample(9.).0, Vec3::ZERO);
        assert_eq!(body(10, 11).sample(12.).0, Vec3::X);
    }

    #[test]
    fn spans_missed_ticks() {
        // the app missed tick 11, so the previous sample is two ticks old
        let (position, _) = body(10, 12).sample(11.);
        assert!((position.x - 0.5).abs() < 1e-5);
    }

    #[test]
    fn new_bodies_use_their_only_sample() {
        assert_eq!(body(10, 10).sample(9.5).0, Vec3::X);
    }
}
