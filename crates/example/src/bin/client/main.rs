use std::time::Duration;

use bevy::{
    color::palettes::css::*,
    log::{Level, LogPlugin},
    prelude::*,
};
use example::{
    networking::StreamHeader,
    scheme::{PhysicsScheme, RequestUpdateExampleBox, UpdateExampleBox},
    simulation::ExampleBox,
};
use nevy::*;
use nevy_prediction::client::prelude::*;

use crate::networking::ClientConnection;

pub mod networking;

fn main() {
    let mut app = App::new();

    app.add_plugins(DefaultPlugins.set(LogPlugin {
        level: Level::DEBUG,
        filter: bevy::log::DEFAULT_FILTER.to_string()
            + ",bevy_render=info,bevy_app=info,offset_allocator=info,bevy_asset=info,gilrs=info,bevy_winit=info",
        ..default()
    }));

    example::build(&mut app);
    networking::build(&mut app);

    app.add_plugins(NevyPredictionClientPlugin::<PhysicsScheme>::default());

    app.insert_resource(PredictionInterval(Duration::from_millis(300)));

    app.add_systems(PostStartup, debug_connect_to_server);
    app.add_systems(Startup, setup_camera);
    app.add_systems(
        Update,
        (
            render_example_boxes.after(StepSimulationSystems),
            simulation_input.in_set(ClientSimulationSystems::QueueUpdatesSystems),
        ),
    );

    app.run();
}

fn debug_connect_to_server(
    mut commands: Commands,
    endpoint_q: Query<Entity, With<networking::ClientEndpoint>>,
) -> Result {
    let endpoint_entity = endpoint_q.single()?;

    let address = std::env::args()
        .nth(1)
        .expect("Expected server address as first argument")
        .parse()
        .expect("Invalid server address");

    commands.spawn((
        ClientConnection,
        PredictionServerConnection,
        nevy::ConnectionOf(endpoint_entity),
        nevy::QuicConnectionConfig {
            client_config: networking::create_connection_config(),
            address,
            server_name: "example.server".to_string(),
        },
    ));

    Ok(())
}

fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(-1., 5., 1.).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn simulation_input(
    input: Res<ButtonInput<KeyCode>>,
    box_q: Query<(&SimulationEntity, &ExampleBox), With<ExampleBox>>,
    mut updates: PredictionUpdateCreator<UpdateExampleBox>,
    mut sender: LocalMessageSender,
    message_id: Res<MessageId<RequestUpdateExampleBox>>,
    server_q: Query<Entity, With<ClientConnection>>,
) -> Result {
    let server_entity = server_q.single()?;

    if !input.just_pressed(KeyCode::Space) {
        return Ok(());
    }

    for (&entity, example_box) in &box_q {
        let update = updates.create(UpdateExampleBox {
            entity,
            example_box: ExampleBox {
                position: example_box.position,
                velocity: example_box.velocity * -1.,
            },
        });

        sender.write(
            StreamHeader::Messages,
            server_entity,
            *message_id,
            false, // Don't send if congested
            &RequestUpdateExampleBox { update },
        )?;

        debug!("Sent input for {:?}", entity);
    }

    Ok(())
}

fn render_example_boxes(mut gizmos: Gizmos, box_q: Query<&ExampleBox>) {
    gizmos.circle(
        Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
        1.,
        ORANGE,
    );

    for example_box in &box_q {
        gizmos.cuboid(
            Transform::from_translation(
                Quat::from_rotation_y(example_box.position).mul_vec3(Vec3::NEG_Z),
            ),
            WHITE,
        );
    }
}
