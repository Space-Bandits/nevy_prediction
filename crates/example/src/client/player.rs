//! Controls the local player with the keyboard or a bot, and renders players.

use bevy::{color::palettes::css::*, prelude::*};
use nevy_prediction::prelude::*;

use crate::{
    client::{
        ClientRecorder, Config,
        networking::{ClientMessages, LocalClientMessageSender},
    },
    simulation::{
        drones::DroneState,
        props::{ARENA_HALF_SIZE, Prop},
        player::{Player, PlayerInput, PlayerState, RequestMovePlayer, SetLocalPlayer},
        workload::{hash, unit},
    },
};

pub fn build(app: &mut App) {
    app.add_systems(
        Update,
        (
            set_local_player.before(ClientSimulationSystems::QueueUpdates),
            update_player_input.in_set(ClientSimulationSystems::QueueUpdates),
            (record_players, update_render_history, cast_rays)
                .after(SyncSimulationSystems),
        ),
    );

    if app.world().resource::<Config>().0.window {
        app.add_systems(
            Update,
            render.after(update_render_history),
        );
        app.add_systems(
            PostUpdate,
            render_props.after(nevy_prediction_avian::prelude::PhysicsPresentationSystems::Follow),
        );
    }
}

#[derive(Resource, Deref)]
pub struct LocalPlayer(pub SimulationEntity);

fn set_local_player(mut commands: Commands, mut messages: ClientMessages<SetLocalPlayer>) {
    for SetLocalPlayer { entity } in messages.drain() {
        commands.insert_resource(LocalPlayer(entity));

        debug!("The local player is: {:?}", entity);
    }
}

/// Chooses random inputs for headless clients.
struct Bot {
    seed: u64,
    changes: u64,
    next_change: f64,
    input: PlayerInput,
}

impl Bot {
    fn update(&mut self, now: f64) -> &PlayerInput {
        if now < self.next_change {
            return &self.input;
        }

        let roll = |offset: u64| unit(hash(self.seed) ^ (self.changes * 8 + offset));

        // mostly moving, sometimes standing still
        self.input = PlayerInput {
            forward: roll(0) < 0.4,
            backward: roll(1) < 0.4,
            left: roll(2) < 0.4,
            right: roll(3) < 0.4,
        };
        self.next_change = now + 0.2 + roll(4) as f64 * 0.8;
        self.changes += 1;

        &self.input
    }
}

fn update_player_input(
    config: Res<Config>,
    keyboard_input: Option<Res<ButtonInput<KeyCode>>>,
    real_time: Res<Time<Real>>,
    mut bot: Local<Option<Bot>>,
    mut last_input: Local<PlayerInput>,
    local_player: Option<Res<LocalPlayer>>,
    mut updates: PredictionUpdateCreator<UpdateComponent<PlayerInput>>,
    mut messages: LocalClientMessageSender,
    mut recorder: ResMut<ClientRecorder>,
) -> Result {
    messages.flush()?;

    let player_input = match (config.0.bot, keyboard_input) {
        (Some(seed), _) => bot
            .get_or_insert_with(|| Bot {
                seed,
                changes: 0,
                next_change: 0.,
                input: default(),
            })
            .update(real_time.elapsed_secs_f64())
            .clone(),
        (None, Some(keyboard_input)) => PlayerInput {
            forward: keyboard_input.pressed(KeyCode::KeyW),
            backward: keyboard_input.pressed(KeyCode::KeyS),
            left: keyboard_input.pressed(KeyCode::KeyA),
            right: keyboard_input.pressed(KeyCode::KeyD),
        },
        (None, None) => return Ok(()),
    };

    let Some(local_player) = local_player else {
        return Ok(());
    };

    if *last_input == player_input {
        return Ok(());
    }

    *last_input = player_input.clone();

    let update = updates.create(UpdateComponent {
        entity: **local_player,
        component: player_input.clone(),
    });

    recorder.inputs_sent += 1;

    messages.write(
        true,
        &RequestMovePlayer {
            tick: update.tick,
            input: player_input,
        },
    )?;

    Ok(())
}

/// Records the position of every player for every tick that is extracted, to compare with the server.
fn record_players(
    config: Res<Config>,
    time: Res<Time<SimulationTime>>,
    mut last_tick: Local<Option<SimulationTick>>,
    local_player: Option<Res<LocalPlayer>>,
    player_q: Query<(&SimulationEntity, &PlayerState), With<Player>>,
    prop_q: Query<(&SimulationEntity, &avian3d::prelude::Position), With<Prop>>,
) {
    let Some(sink) = &config.0.report else {
        return;
    };

    // Recorded whenever the state changes, a finished prediction can correct a tick that was already extracted.
    let tick = time.current_tick();
    if *last_tick == Some(tick) && !time.is_changed() {
        return;
    }
    *last_tick = Some(tick);

    let mut reports = sink.lock();
    for (&entity, state) in &player_q {
        let local = local_player.as_ref().is_some_and(|local| ***local == entity);
        reports
            .truth
            .record_client(config.0.index, local, entity, tick, state.position);
    }
    for (&entity, position) in &prop_q {
        reports
            .truth
            .record_client_prop(config.0.index, entity, tick, position.0);
    }
}

/// The two latest simulation states of a player, for interpolating between them.
#[derive(Component)]
pub struct RenderHistory {
    previous: (SimulationTick, Vec2),
    current: (SimulationTick, Vec2),
}

impl RenderHistory {
    /// Interpolates between the two states at a simulation time.
    fn position(&self, time: &Time<PredictionTime>, step: f32) -> Vec2 {
        let previous = self.previous.0.0 as f32 * step;
        let current = self.current.0.0 as f32 * step;

        if current <= previous {
            return self.current.1;
        }

        let s = ((time.elapsed_secs() - previous) / (current - previous)).clamp(0., 1.);
        self.previous.1.lerp(self.current.1, s)
    }
}

fn update_render_history(
    mut commands: Commands,
    time: Res<Time<SimulationTime>>,
    mut player_q: Query<(Entity, &PlayerState, Option<&mut RenderHistory>), With<Player>>,
) {
    let tick = time.current_tick();

    for (entity, state, history) in &mut player_q {
        match history {
            Some(mut history) => {
                if history.current.0 != tick {
                    history.previous = history.current;
                }

                // A finished prediction can correct the state of a tick that was already extracted.
                history.current = (tick, state.position);
            }
            None => {
                commands.entity(entity).insert(RenderHistory {
                    previous: (tick, state.position),
                    current: (tick, state.position),
                });
            }
        }
    }
}

fn render(
    mut gizmos: Gizmos,
    time: Res<Time<PredictionTime>>,
    local_player: Option<Res<LocalPlayer>>,
    player_q: Query<(&SimulationEntity, &RenderHistory)>,
    drone_q: Query<&DroneState>,
) {
    let step = crate::simulation::PhysicsScheme::step_interval().as_secs_f32();

    for (&entity, history) in &player_q {
        let position = history.position(&time, step);
        let color = if local_player.as_ref().is_some_and(|local| ***local == entity) {
            LIME
        } else {
            WHITE
        };

        gizmos.cube(
            Transform::from_xyz(position.x, 0., position.y),
            color,
        );
    }

    for drone in &drone_q {
        gizmos.sphere(
            Isometry3d::from_translation(Vec3::new(drone.position.x, 0.5, drone.position.y)),
            0.1,
            ORANGE,
        );
    }
}

/// Draws props where they were interpolated to.
fn render_props(mut gizmos: Gizmos, prop_q: Query<(&Prop, &Transform)>) {
    gizmos.cube(
        Transform::from_scale(Vec3::splat(ARENA_HALF_SIZE * 2.)),
        GRAY,
    );

    for (prop, transform) in &prop_q {
        gizmos.cube(
            transform.with_scale(Vec3::splat(prop.half_size * 2.)),
            AQUA,
        );
    }
}

/// Casts a ray against the app's world, which never steps physics,
/// to check that spatial queries see the extracted props and walls.
fn cast_rays(
    mut recorder: ResMut<ClientRecorder>,
    spatial_query: avian3d::prelude::SpatialQuery,
    wall_q: Query<(), With<crate::simulation::props::Wall>>,
    time: Res<Time<Real>>,
) {
    if wall_q.is_empty() {
        return;
    }

    let angle = time.elapsed_secs() * 3.;
    let direction = Dir3::new(Vec3::new(angle.cos(), 0.3, angle.sin())).unwrap();

    recorder.raycasts += 1;
    if spatial_query
        .cast_ray(Vec3::ZERO, direction, ARENA_HALF_SIZE * 4., true, &default())
        .is_some()
    {
        recorder.raycast_hits += 1;
    }
}
