//! The [`UpdateRigidBody`] world update, which corrects a body to the server's state.

use avian3d::prelude::*;
use bevy::prelude::*;
use nevy_prediction::prelude::*;
use serde::{Deserialize, Serialize};

use crate::PredictionPhysicsSystems;

pub(crate) fn build(app: &mut App, instance: SimulationInstance) {
    app.add_world_update::<UpdateRigidBody>();

    if instance.steps() {
        app.add_systems(
            SimulationUpdate,
            apply_rigid_body_updates.in_set(PredictionPhysicsSystems::ApplyCorrections),
        );
    }
}

/// Sets the state of a rigid body.
///
/// The server sends these to correct the clients' copies of its bodies, see [`RigidBodySyncPlugin`](crate::server::RigidBodySyncPlugin).
/// They are written straight to [`Position`], [`Rotation`], the velocities and [`Transform`] before physics runs.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct UpdateRigidBody {
    pub entity: SimulationEntity,
    pub position: Vec3,
    pub rotation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
}

impl UpdateRigidBody {
    /// Creates an update from the state of a body, such as one in the server's world.
    pub fn from_body(
        entity: SimulationEntity,
        position: &Position,
        rotation: &Rotation,
        linear_velocity: &LinearVelocity,
        angular_velocity: &AngularVelocity,
    ) -> Self {
        UpdateRigidBody {
            entity,
            position: position.0,
            rotation: rotation.0,
            linear_velocity: linear_velocity.0,
            angular_velocity: angular_velocity.0,
        }
    }
}

/// Also writes the body's [`Transform`] to match.
///
/// avian copies [`GlobalTransform`] into a [`Position`] that was added since physics last ran,
/// so without this a correction applied on the tick its body spawned would be replaced by the spawn's [`Transform`].
fn apply_rigid_body_updates(
    mut updates: ReadyUpdates<UpdateRigidBody>,
    map: Res<SimulationEntityMap>,
    mut body_q: Query<(
        &mut Position,
        &mut Rotation,
        &mut LinearVelocity,
        &mut AngularVelocity,
        &mut Transform,
        Option<&ChildOf>,
    )>,
    parent_q: Query<&GlobalTransform>,
) {
    for update in updates.drain() {
        // The body may have been despawned by an update the server applied on the same tick.
        let Some(local_entity) = map.get(update.entity) else {
            debug!(
                "{} didn't exist when its rigid body was corrected",
                update.entity
            );
            continue;
        };

        let Ok((
            mut position,
            mut rotation,
            mut linear_velocity,
            mut angular_velocity,
            mut transform,
            child_of,
        )) = body_q.get_mut(local_entity)
        else {
            warn!(
                "{} wasn't a rigid body when it was corrected",
                update.entity
            );
            continue;
        };

        position.0 = update.position;
        rotation.0 = update.rotation;
        linear_velocity.0 = update.linear_velocity;
        angular_velocity.0 = update.angular_velocity;

        let global_transform = GlobalTransform::from(
            Transform::from_translation(update.position).with_rotation(update.rotation),
        );
        let local_transform = match child_of.and_then(|child_of| parent_q.get(child_of.0).ok()) {
            Some(parent_transform) => global_transform.reparented_to(parent_transform),
            None => global_transform.compute_transform(),
        };
        transform.translation = local_transform.translation;
        transform.rotation = local_transform.rotation;
    }
}
