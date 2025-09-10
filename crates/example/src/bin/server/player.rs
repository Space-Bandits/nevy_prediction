use bevy::prelude::*;
use example::{
    networking::StreamHeader,
    simulation::player::{
        Player, PlayerInput, PlayerState, RequestMovePlayer, SetLocalPlayer, SpawnPlayer,
    },
};
use nevy::{LocalMessageSender, MessageId, ReceivedMessages};
use nevy_prediction::server::prelude::*;

use crate::{SimulationEntityAllocator, new_pairs::NewPairs, state::JoinedClient};

pub fn build(app: &mut App) {
    app.add_systems(
        Update,
        (
            (spawn_players, init_players)
                .chain()
                .in_set(ServerSimulationSystems::QueueUpdatesSystems),
            accept_move_players,
        ),
    );
}

#[derive(Component)]
struct ClientPlayer {
    player_entity: Entity,
}

fn spawn_players(
    mut commands: Commands,
    client_q: Query<Entity, Added<JoinedClient>>,
    mut allocator: ResMut<SimulationEntityAllocator>,
    mut messages: LocalMessageSender,
    message_id: Res<MessageId<SetLocalPlayer>>,
) -> Result {
    for client_entity in client_q.iter() {
        let entity = allocator.next();

        let player_entity = commands.spawn((entity, Player)).id();

        commands
            .entity(client_entity)
            .insert(ClientPlayer { player_entity });

        messages.write(
            StreamHeader::Messages,
            client_entity,
            *message_id,
            true,
            &SetLocalPlayer { entity },
        )?;
    }

    Ok(())
}

fn init_players(
    pairs: NewPairs<PredictionClient, Player>,
    player_q: Query<(&SimulationEntity, &PlayerInput, &PlayerState)>,

    mut updates: WorldUpdateSender,
    spawn_player: Res<MessageId<ServerWorldUpdate<SpawnPlayer>>>,
    update_input: Res<MessageId<ServerWorldUpdate<UpdateComponent<PlayerInput>>>>,
    update_state: Res<MessageId<ServerWorldUpdate<UpdateComponent<PlayerState>>>>,
) -> Result {
    for (client_entity, player_entity) in &pairs {
        let (&entity, player_input, player_state) = player_q.get(player_entity)?;

        updates.write_now(
            StreamHeader::Messages,
            client_entity,
            *spawn_player,
            true,
            SpawnPlayer { entity: entity },
        )?;

        updates.write_now(
            StreamHeader::Messages,
            client_entity,
            *update_input,
            true,
            UpdateComponent {
                entity: entity,
                component: player_input.clone(),
            },
        )?;

        updates.write_now(
            StreamHeader::Messages,
            client_entity,
            *update_state,
            true,
            UpdateComponent {
                entity: entity,
                component: player_state.clone(),
            },
        )?;
    }

    Ok(())
}

fn accept_move_players(
    mut requesting_client_q: Query<(&ClientPlayer, &mut ReceivedMessages<RequestMovePlayer>)>,
    player_q: Query<&SimulationEntity>,
    client_q: Query<Entity, With<PredictionClient>>,
    mut queue: ResMut<WorldUpdateQueue<UpdateComponent<PlayerInput>>>,
    mut sender: WorldUpdateSender,
    message_id: Res<MessageId<ServerWorldUpdate<UpdateComponent<PlayerInput>>>>,
) -> Result {
    for (&ClientPlayer { player_entity }, mut messages) in &mut requesting_client_q {
        for RequestMovePlayer { time, input } in messages.drain() {
            let &player_simulation_entity = player_q.get(player_entity)?;

            let update = WorldUpdate {
                time,
                update: UpdateComponent {
                    entity: player_simulation_entity,
                    component: input,
                },
            };

            for client_entity in &client_q {
                sender.write(
                    StreamHeader::Messages,
                    client_entity,
                    *message_id,
                    true,
                    update.clone(),
                )?;
            }

            queue.insert(update);
        }
    }

    Ok(())
}
