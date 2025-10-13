use bevy::{ecs::system::SystemParam, prelude::*};
use nevy::*;
use serde::Serialize;

use crate::networking::ClientConnection;

/// Utility parameter for receiving messages on the client connection.
#[derive(SystemParam)]
pub struct ClientMessages<'w, 's, T>
where
    T: Send + Sync + 'static,
{
    connection_q: Query<'w, 's, &'static mut ReceivedNetMessages<T>, With<ClientConnection>>,
}

impl<'w, 's, T> ClientMessages<'w, 's, T>
where
    T: Send + Sync + 'static,
{
    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        let mut messages = Vec::new();

        if let Ok(mut received_messages) = self.connection_q.single_mut() {
            for message in received_messages.drain() {
                messages.push(message);
            }
        }

        messages.into_iter()
    }
}

#[derive(SystemParam)]
pub struct LocalClientMessageSender<'w, 's> {
    client_q: Query<'w, 's, Entity, With<ClientConnection>>,
    sender: LocalNetMessageSender<'w, 's>,
}

impl<'w, 's> LocalClientMessageSender<'w, 's> {
    pub fn flush(&mut self) -> Result {
        self.sender.flush()
    }

    pub fn write<T>(
        &mut self,
        message_id: NetMessageId<T>,
        queue: bool,
        message: &T,
    ) -> Result<bool>
    where
        T: Serialize,
    {
        let client_entity = self.client_q.single()?;

        self.sender
            .write::<T>(client_entity, message_id, queue, message)
    }

    pub fn finish_if_uncongested(&mut self) -> Result {
        self.sender.finish_all_if_uncongested()
    }
}

#[derive(SystemParam)]
pub struct SharedClientMessageSender<'w, 's, S>
where
    S: Send + Sync + 'static,
{
    client_q: Query<'w, 's, Entity, With<ClientConnection>>,
    sender: SharedNetMessageSender<'w, 's, S>,
}

impl<'w, 's, S> SharedClientMessageSender<'w, 's, S>
where
    S: Send + Sync + 'static,
{
    pub fn write<T>(
        &mut self,
        message_id: NetMessageId<T>,
        queue: bool,
        message: &T,
    ) -> Result<bool>
    where
        T: Serialize,
    {
        let client_entity = self.client_q.single()?;

        self.sender
            .write::<T>(client_entity, message_id, queue, message)
    }

    pub fn finish_if_uncongested(&mut self) -> Result {
        self.sender.finish_all_if_uncongested()
    }
}
