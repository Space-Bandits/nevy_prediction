use bevy::prelude::*;
use nevy::*;

pub mod initialize_pairs;

pub fn build(app: &mut App) {
    app.add_systems(Update, join_clients);
}

/// Marker component for connection entities that have joined the game and should receive game updates.
#[derive(Component)]
pub struct JoinedClient;

fn join_clients(
    mut commands: Commands,
    mut connection_q: Query<(Entity, &ConnectionStatus), Changed<ConnectionStatus>>,
) {
    for (connection_entity, status) in connection_q.iter_mut() {
        match status {
            ConnectionStatus::Established => {
                commands.entity(connection_entity).insert(JoinedClient);
            }
            _ => {
                commands.entity(connection_entity).remove::<JoinedClient>();
            }
        }
    }
}
