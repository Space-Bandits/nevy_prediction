use std::marker::PhantomData;

use bevy::{
    ecs::{component::Mutable, intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};
use nevy::*;

use crate::common::{PredictionScheme, WorldUpdate};

pub struct NevyPredictionClientPlugin<S> {
    pub _p: PhantomData<S>,
    pub schedule: Interned<dyn ScheduleLabel>,
}

impl<S> Default for NevyPredictionClientPlugin<S> {
    fn default() -> Self {
        NevyPredictionClientPlugin {
            _p: PhantomData,
            schedule: Update.intern(),
        }
    }
}

impl<S> Plugin for NevyPredictionClientPlugin<S>
where
    S: PredictionScheme,
{
    fn build(&self, app: &mut App) {
        crate::common::build::<S>(app);

        app.configure_sets(
            self.schedule,
            (ReceiveUpdatesSet::Updates, ReceiveUpdatesSet::Sync).chain(),
        );

        for update in S::updates().0 {
            update.build_client(app, self.schedule);
        }
    }
}

pub(crate) fn build_message<S, T>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    S: Send + Sync + 'static,
    T: Send + Sync + 'static,
{
    app.insert_resource(WorldUpdates::<S, T> {
        _p: PhantomData,
        updates: Vec::new(),
    });

    app.add_systems(
        schedule,
        receive_world_updates::<S, T>.in_set(ReceiveUpdatesSet::Updates),
    );
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ReceiveUpdatesSet {
    Updates,
    Sync,
}

#[derive(Resource)]
struct WorldUpdates<S, T> {
    _p: PhantomData<S>,
    updates: Vec<T>,
}

fn receive_world_updates<S, T>(
    mut received_messages: Query<&mut ReceivedMessages<WorldUpdate<S, T>>>,
    mut messages: ResMut<WorldUpdates<S, T>>,
) where
    ReceivedMessages<WorldUpdate<S, T>>: Component<Mutability = Mutable>,
    WorldUpdates<S, T>: Resource,
{
    for mut received_messages in &mut received_messages {
        for WorldUpdate { update, .. } in received_messages.drain() {
            messages.updates.push(update);

            debug!("received a world update");
        }
    }
}
