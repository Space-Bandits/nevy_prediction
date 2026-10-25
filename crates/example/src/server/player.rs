//! Spawns a player for every client and applies their inputs.
//!
//! Every change to the simulation is queued with a [`ServerUpdateQueue`] and broadcast to clients,
//! because the simulation runs on its own thread and the app's world only holds a copy of it.

use bevy::prelude::*;
use nevy::prelude::*;
use nevy_prediction::prelude::*;

use crate::{
    server::SimulationEntityAllocator,
    simulation::player::{
        Player, PlayerInput, PlayerState, RequestMovePlayer, SetLocalPlayer, SpawnPlayer,
    },
};

pub fn build(app: &mut App) {
    app.add_systems(
        Update,
        (
            join_clients.before(ServerSimulationSystems::SendResets),
            (init_players, spawn_players, send_local_players, accept_move_players)
                .chain()
                .in_set(ServerSimulationSystems::QueueUpdates),
        ),
    );

    app.add_observer(despawn_players);
}

/// Inserted on connections whose player has been spawned.
#[derive(Component)]
pub struct ClientPlayer {
    pub entity: SimulationEntity,
}

/// Clients join prediction as soon as their connection is established.
fn join_clients(
    mut commands: Commands,
    connection_q: Query<(Entity, &ConnectionStatus, Has<PredictionClient>), Changed<ConnectionStatus>>,
) {
    for (connection_entity, status, joined) in &connection_q {
        match status {
            ConnectionStatus::Established if !joined => {
                commands.entity(connection_entity).insert(PredictionClient);
            }
            ConnectionStatus::Established => {}
            _ => {
                commands
                    .entity(connection_entity)
                    .remove::<(PredictionClient, ClientPlayer)>();
            }
        }
    }
}

/// Sends the players that are in the app's world to new clients.
///
/// Players that have been queued but haven't reached the app's world yet are replayed to the client automatically.
fn init_players(
    client_q: Query<Entity, Added<PredictionClient>>,
    player_q: Query<(&SimulationEntity, &PlayerInput, &PlayerState), With<Player>>,
    mut updates: WorldUpdateSender,
) -> Result {
    for client_entity in &client_q {
        for (&entity, input, state) in &player_q {
            updates.write_now(client_entity, true, SpawnPlayer { entity })?;

            updates.write_now(
                client_entity,
                true,
                UpdateComponent {
                    entity,
                    component: input.clone(),
                },
            )?;

            updates.write_now(
                client_entity,
                true,
                UpdateComponent {
                    entity,
                    component: state.clone(),
                },
            )?;
        }
    }

    Ok(())
}

/// Spawns a player for every new client.
pub fn spawn_players(
    mut commands: Commands,
    new_client_q: Query<Entity, Added<PredictionClient>>,
    client_q: Query<Entity, With<PredictionClient>>,
    mut allocator: ResMut<SimulationEntityAllocator>,
    mut queue: ServerUpdateQueue<SpawnPlayer>,
    mut updates: WorldUpdateSender,
) -> Result {
    for new_client_entity in &new_client_q {
        let entity = allocator.next();

        let update = queue.queue_now(SpawnPlayer { entity });

        for client_entity in &client_q {
            updates.write(client_entity, true, true, update.clone())?;
        }

        commands
            .entity(new_client_entity)
            .insert(ClientPlayer { entity });
    }

    Ok(())
}

/// Tells clients which player is theirs.
fn send_local_players(
    client_q: Query<(Entity, &ClientPlayer), Added<ClientPlayer>>,
    mut messages: LocalMessageSender,
) -> Result {
    messages.flush()?;

    for (client_entity, client_player) in &client_q {
        messages.write(
            client_entity,
            true,
            &SetLocalPlayer {
                entity: client_player.entity,
            },
        )?;
    }

    Ok(())
}

fn accept_move_players(
    mut requesting_client_q: Query<(
        Entity,
        &ClientPlayer,
        &mut ReceivedMessages<RequestMovePlayer>,
    )>,
    client_q: Query<Entity, With<PredictionClient>>,
    mut queue: ServerUpdateQueue<UpdateComponent<PlayerInput>>,
    mut updates: WorldUpdateSender,
) -> Result {
    for (requesting_client_entity, client_player, mut messages) in &mut requesting_client_q {
        for RequestMovePlayer { tick, input } in messages.drain() {
            // If this arrived late it is moved to a later tick, which is what every client is told.
            let update = queue.queue(WorldUpdate {
                tick,
                update: UpdateComponent {
                    entity: client_player.entity,
                    component: input,
                },
            });

            for client_entity in &client_q {
                // The requesting client already predicted its own input.
                let include_in_prediction = client_entity != requesting_client_entity;

                updates.write(client_entity, true, include_in_prediction, update.clone())?;
            }
        }
    }

    Ok(())
}

/// Despawns a client's player when it leaves.
fn despawn_players(
    remove: On<Remove, ClientPlayer>,
    player_q: Query<&ClientPlayer>,
    client_q: Query<Entity, With<PredictionClient>>,
    mut queue: ServerUpdateQueue<DespawnSimulatonEntity>,
    mut updates: WorldUpdateSender,
) -> Result {
    let entity = player_q.get(remove.entity)?.entity;

    let update = queue.queue_now(DespawnSimulatonEntity { entity });

    for client_entity in &client_q {
        if client_entity != remove.entity {
            updates.write(client_entity, true, true, update.clone())?;
        }
    }

    Ok(())
}
