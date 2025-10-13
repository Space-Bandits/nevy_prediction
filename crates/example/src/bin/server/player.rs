use bevy::prelude::*;
use example::simulation::player::{
    Player, PlayerInput, PlayerState, RequestMovePlayer, SetLocalPlayer, SpawnPlayer,
};
use nevy::*;
use nevy_prediction::prelude::*;

use crate::{SimulationEntityAllocator, new_pairs::NewPairs, state::JoinedClient};

pub fn build(app: &mut App) {
    app.add_systems(
        Update,
        (
            (spawn_players, init_players)
                .chain()
                .in_set(ServerSimulationSystems::QueueUpdates),
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
    mut messages: LocalNetMessageSender,
    message_id: Res<NetMessageId<SetLocalPlayer>>,
) -> Result {
    for client_entity in client_q.iter() {
        let entity = allocator.next();

        let player_entity = commands.spawn((entity, Player)).id();

        commands
            .entity(client_entity)
            .insert(ClientPlayer { player_entity });

        messages.write(client_entity, *message_id, true, &SetLocalPlayer { entity })?;
    }

    Ok(())
}

fn init_players(
    pairs: NewPairs<PredictionClient, Player>,
    player_q: Query<(&SimulationEntity, &PlayerInput, &PlayerState)>,

    mut updates: WorldUpdateSender,
    spawn_player: Res<NetMessageId<ServerWorldUpdate<SpawnPlayer>>>,
    update_input: Res<NetMessageId<ServerWorldUpdate<UpdateComponent<PlayerInput>>>>,
    update_state: Res<NetMessageId<ServerWorldUpdate<UpdateComponent<PlayerState>>>>,
) -> Result {
    for (client_entity, player_entity) in &pairs {
        let (&entity, player_input, player_state) = player_q.get(player_entity)?;

        updates.write_now(
            client_entity,
            *spawn_player,
            true,
            SpawnPlayer { entity: entity },
        )?;

        updates.write_now(
            client_entity,
            *update_input,
            true,
            UpdateComponent {
                entity: entity,
                component: player_input.clone(),
            },
        )?;

        updates.write_now(
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
    mut requesting_client_q: Query<(
        Entity,
        &ClientPlayer,
        &mut ReceivedNetMessages<RequestMovePlayer>,
    )>,
    player_q: Query<&SimulationEntity>,
    client_q: Query<Entity, With<PredictionClient>>,
    mut queue: ResMut<UpdateExecutionQueue<UpdateComponent<PlayerInput>>>,
    mut sender: WorldUpdateSender,
    message_id: Res<NetMessageId<ServerWorldUpdate<UpdateComponent<PlayerInput>>>>,
) -> Result {
    for (requesting_client_entity, &ClientPlayer { player_entity }, mut messages) in
        &mut requesting_client_q
    {
        for RequestMovePlayer { tick, input } in messages.drain() {
            let &player_simulation_entity = player_q.get(player_entity)?;

            let update = WorldUpdate {
                tick,
                update: UpdateComponent {
                    entity: player_simulation_entity,
                    component: input,
                },
            };

            for client_entity in &client_q {
                sender.write(
                    client_entity,
                    *message_id,
                    true,
                    client_entity != requesting_client_entity,
                    update.clone(),
                )?;
            }

            queue.insert(update);
        }
    }

    Ok(())
}
