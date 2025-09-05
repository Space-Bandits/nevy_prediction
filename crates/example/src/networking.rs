use bevy::prelude::*;
use nevy::*;

pub fn build(app: &mut App) {
    app.add_plugins((
        NevyPlugin::default(),
        NevyHeaderPlugin::default(),
        NevyMessagesPlugin::new(StreamHeader::Messages),
    ));

    app.add_systems(PostUpdate, log_connection_status.after(UpdateEndpoints));
}

pub enum StreamHeader {
    Messages,
}

impl Into<u16> for StreamHeader {
    fn into(self) -> u16 {
        self as u16
    }
}

pub fn log_connection_status(
    connection_q: Query<
        (Entity, &ConnectionOf, &QuicConnection, &ConnectionStatus),
        Changed<ConnectionStatus>,
    >,
    mut endpoint_q: Query<&mut QuicEndpoint>,
) -> Result {
    for (connection_entity, connection_of, connection, status) in &connection_q {
        let mut endpoint = endpoint_q.get_mut(**connection_of)?;

        let address = endpoint
            .get_connection(connection)
            .map(|connection| connection.get_remote_address());

        match status {
            ConnectionStatus::Connecting => {
                info!("New connection {} addr {:?}", connection_entity, address)
            }
            ConnectionStatus::Established => info!(
                "Connection {} addr {:?} established",
                connection_entity, address
            ),
            ConnectionStatus::Closed { reason } => info!(
                "Connection {} addr {:?} closed: {:?}",
                connection_entity, address, reason
            ),
            ConnectionStatus::Failed { error } => info!(
                "Connection {} addr {:?} failed: {}",
                connection_entity, address, error
            ),
        }
    }

    Ok(())
}
