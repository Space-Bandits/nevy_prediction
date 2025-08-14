use bevy::{
    log::{Level, LogPlugin},
    prelude::*,
};
use example::{NewPhysicsBox, PhysicsScheme, networking::StreamHeader};
use nevy::*;
use nevy_prediction::server::*;

use crate::state::initialize_pairs::InitializePairs;

pub mod networking;
pub mod state;

fn main() {
    let mut app = App::new();

    app.add_plugins(MinimalPlugins);
    app.add_plugins(LogPlugin {
        level: Level::DEBUG,
        filter: bevy::log::DEFAULT_FILTER.to_string()
            + ",bevy_render=info,bevy_app=info,offset_allocator=info,bevy_asset=info,gilrs=info,bevy_winit=info",
        ..default()
    });

    example::build(&mut app);

    networking::build(&mut app);
    state::build(&mut app);

    app.add_plugins(NevyPredictionServerPlugin::<PhysicsScheme>::default());

    app.add_systems(Startup, spawn_boxes);

    app.add_systems(
        Update,
        init_new_boxes.in_set(SchemeUpdateSet::<PhysicsScheme>::SendUpdates),
    );

    app.run();
}

#[derive(Component)]
struct PhysicsBox;

fn spawn_boxes(mut commands: Commands) {
    commands.spawn(PhysicsBox);
}

fn init_new_boxes(
    pairs: InitializePairs<PhysicsBox>,
    mut messages: SharedMessageSender<UpdateSender<PhysicsScheme>>,
    message_id: Res<MessageId<WorldUpdate<PhysicsScheme, NewPhysicsBox>>>,
) -> Result {
    // debug!("Sending updates");
    for (client_entity, box_entity) in pairs.iter() {
        messages.write(
            StreamHeader::Messages,
            client_entity,
            *message_id,
            true,
            &WorldUpdate::new(NewPhysicsBox {
                entity: box_entity.into(),
            }),
        )?;
    }

    Ok(())
}
