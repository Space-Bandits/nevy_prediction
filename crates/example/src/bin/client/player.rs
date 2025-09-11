use bevy::{color::palettes::css::*, prelude::*};
use example::simulation::player::{PlayerInput, PlayerState, RequestMovePlayer, SetLocalPlayer};
use nevy::MessageId;
use nevy_prediction::client::prelude::*;

use crate::networking::params::{ClientMessages, LocalClientMessageSender};

pub fn build(app: &mut App) {
    app.add_systems(
        Update,
        (
            set_local_player,
            render_players.after(StepSimulationSystems),
            update_player_input,
        ),
    );
}

#[derive(Resource, Deref)]
pub struct LocalPlayer(pub SimulationEntity);

fn set_local_player(mut commands: Commands, mut messages: ClientMessages<SetLocalPlayer>) {
    for SetLocalPlayer { entity } in messages.drain() {
        commands.insert_resource(LocalPlayer(entity));

        debug!("The local player is: {:?}", entity);
    }
}

fn render_players(mut gizmos: Gizmos, player_q: Query<&PlayerState>) {
    for player_state in &player_q {
        gizmos.cuboid(
            Transform::from_xyz(player_state.position.x, 0., player_state.position.y),
            WHITE,
        );
    }
}

fn update_player_input(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut last_input: Local<PlayerInput>,
    local_player: Option<Res<LocalPlayer>>,
    mut updates: PredictionUpdateCreator<UpdateComponent<PlayerInput>>,
    mut messages: LocalClientMessageSender,
    message_id: Res<MessageId<RequestMovePlayer>>,
) -> Result {
    messages.flush()?;

    let player_input = PlayerInput {
        forward: keyboard_input.pressed(KeyCode::KeyW),
        backward: keyboard_input.pressed(KeyCode::KeyS),
        left: keyboard_input.pressed(KeyCode::KeyA),
        right: keyboard_input.pressed(KeyCode::KeyD),
    };

    if *last_input == player_input {
        return Ok(());
    }

    *last_input = player_input.clone();

    let Some(local_player) = local_player else {
        return Ok(());
    };

    let player_simulation_entity = **local_player;

    let update = updates.create(UpdateComponent {
        entity: player_simulation_entity,
        component: player_input.clone(),
    });

    debug!("Moving: {}", player_input.movement_vector());

    messages.write(
        *message_id,
        false,
        &RequestMovePlayer {
            time: update.time,
            input: player_input,
        },
    )?;

    Ok(())
}
