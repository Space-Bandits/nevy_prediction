//! Correcting the clients' bodies from the server.

use std::time::Duration;

use avian3d::prelude::*;
use bevy::prelude::*;
use nevy_prediction::prelude::*;

use crate::correction::UpdateRigidBody;

/// Add this to the server app to send [`UpdateRigidBody`] corrections to clients.
///
/// - New clients are sent the state of every body when they join.
/// - Every body is sent to every client once per [`RigidBodySyncSettings::interval`],
///   spread out over the interval so they aren't all sent at once.
///
/// Bodies with [`NoRigidBodySync`] are skipped.
/// Disable [`RigidBodySyncSettings::enabled`] to send corrections yourself,
/// for example to only send clients the bodies near them.
pub struct RigidBodySyncPlugin;

impl Plugin for RigidBodySyncPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RigidBodySyncSettings>();

        app.add_systems(
            Update,
            (sync_new_clients, sync_bodies)
                .run_if(|settings: Res<RigidBodySyncSettings>| settings.enabled)
                .in_set(ServerSimulationSystems::QueueUpdates),
        );
    }
}

#[derive(Resource, Clone, Debug)]
pub struct RigidBodySyncSettings {
    pub enabled: bool,
    /// How much simulation time passes between corrections of a body.
    ///
    /// Bodies in contact diverge between corrections, and crowds of colliding bodies diverge chaotically.
    /// In the example's arena of 300 bouncing boxes, correcting every second let props drift 3.5m apart at p99,
    /// while every 250ms kept them within 2mm.
    pub interval: Duration,
    /// Don't send a body again if its state is exactly what was sent last time,
    /// such as a body at rest.
    /// New clients are always sent every body.
    pub skip_unchanged: bool,
}

impl Default for RigidBodySyncSettings {
    fn default() -> Self {
        RigidBodySyncSettings {
            enabled: true,
            interval: Duration::from_millis(250),
            skip_unchanged: true,
        }
    }
}

/// Insert on a body in the server app to stop [`RigidBodySyncPlugin`] correcting it.
#[derive(Component, Default)]
pub struct NoRigidBodySync;

#[derive(Component)]
struct RigidBodySyncState {
    next: Duration,
    last: Option<UpdateRigidBody>,
}

type BodyQueryData = (
    &'static SimulationEntity,
    &'static Position,
    &'static Rotation,
    &'static LinearVelocity,
    &'static AngularVelocity,
);

type BodyQueryFilter = (With<RigidBody>, Without<NoRigidBodySync>);

fn sync_new_clients(
    client_q: Query<Entity, Added<PredictionClient>>,
    body_q: Query<BodyQueryData, BodyQueryFilter>,
    mut sender: WorldUpdateSender,
) -> Result {
    for client_entity in &client_q {
        for (&entity, position, rotation, linear_velocity, angular_velocity) in &body_q {
            let update = UpdateRigidBody::from_body(
                entity,
                position,
                rotation,
                linear_velocity,
                angular_velocity,
            );

            // this is the client's initial state, so it can't be dropped
            sender.write_now(client_entity, true, update)?;
        }
    }

    Ok(())
}

fn sync_bodies(
    mut commands: Commands,
    settings: Res<RigidBodySyncSettings>,
    time: Res<Time<PredictionTime>>,
    mut body_q: Query<(Entity, BodyQueryData, Option<&mut RigidBodySyncState>), BodyQueryFilter>,
    client_q: Query<Entity, With<PredictionClient>>,
    mut sender: WorldUpdateSender,
) -> Result {
    let now = time.elapsed();

    for (body_entity, (&entity, position, rotation, linear_velocity, angular_velocity), state) in
        &mut body_q
    {
        let Some(mut state) = state else {
            commands.entity(body_entity).insert(RigidBodySyncState {
                next: now + settings.interval.mul_f64(phase(entity)),
                last: None,
            });
            continue;
        };

        if now < state.next {
            continue;
        }

        state.next += settings.interval;
        if state.next <= now {
            state.next = now + settings.interval;
        }

        let update = UpdateRigidBody::from_body(
            entity,
            position,
            rotation,
            linear_velocity,
            angular_velocity,
        );

        if settings.skip_unchanged && state.last == Some(update) {
            continue;
        }
        state.last = Some(update);

        for client_entity in &client_q {
            // a dropped correction is replaced by the next one
            sender.write_now(client_entity, false, update)?;
        }
    }

    Ok(())
}

/// Spreads bodies over the sync interval, from 0 to 1.
fn phase(entity: SimulationEntity) -> f64 {
    let hash = entity.0.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (hash >> 11) as f64 / (1u64 << 53) as f64
}
