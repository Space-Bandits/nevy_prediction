use std::{marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};
use nevy::*;

use crate::common::{PredictionScheme, WorldUpdateSync};

pub use crate::common::WorldUpdate;

pub struct NevyPredictionServerPlugin<S> {
    pub _p: PhantomData<S>,
    pub update_interval: Duration,
    pub schedule: Interned<dyn ScheduleLabel>,
}

impl<S> Default for NevyPredictionServerPlugin<S> {
    fn default() -> Self {
        NevyPredictionServerPlugin {
            _p: PhantomData,
            update_interval: Duration::from_millis(200),
            schedule: Update.intern(),
        }
    }
}

impl<S> Plugin for NevyPredictionServerPlugin<S>
where
    S: PredictionScheme,
{
    fn build(&self, app: &mut App) {
        crate::common::build::<S>(app);

        app.configure_sets(
            Update,
            (
                SchemeUpdateSet::<S>::SendUpdates,
                SchemeUpdateSet::<S>::MarkComplete,
            )
                .chain()
                .run_if(world_update_condition(self.update_interval)),
        );

        app.add_shared_sender::<UpdateSender<S>>();

        app.add_systems(
            self.schedule,
            send_update_syncs::<S>.in_set(SchemeUpdateSet::<S>::MarkComplete),
        );
    }
}

fn world_update_condition(interval: Duration) -> impl Condition<()> {
    IntoSystem::into_system(move |mut last_run: Local<Duration>, time: Res<Time>| {
        if *last_run + interval <= time.elapsed() {
            *last_run = time.elapsed();
            true
        } else {
            false
        }
    })
}

#[derive(SystemSet)]
pub enum SchemeUpdateSet<S> {
    Phantom(PhantomData<S>),
    /// This is the system set where world updates are sent to clients.
    ///
    /// This system set is added by [NevyPredictionServerPlugin<S>] and
    /// will only run on an interval for sending updates.
    ///
    /// During this set, send [WorldUpdate]s on the [SharedMessageSender<UpdateSender<S>>]
    SendUpdates,
    MarkComplete,
}

impl<S> Default for SchemeUpdateSet<S> {
    fn default() -> Self {
        SchemeUpdateSet::Phantom(PhantomData)
    }
}

impl<S> Clone for SchemeUpdateSet<S> {
    fn clone(&self) -> Self {
        match self {
            SchemeUpdateSet::Phantom(_) => SchemeUpdateSet::Phantom(PhantomData),
            SchemeUpdateSet::SendUpdates => SchemeUpdateSet::SendUpdates,
            SchemeUpdateSet::MarkComplete => SchemeUpdateSet::SendUpdates,
        }
    }
}

impl<S> Copy for SchemeUpdateSet<S> {}

impl<S> std::fmt::Debug for SchemeUpdateSet<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Phantom(arg0) => f.debug_tuple("Phantom").field(arg0).finish(),
            Self::SendUpdates => write!(f, "SendUpdates"),
            Self::MarkComplete => write!(f, "MarkComplete"),
        }
    }
}

impl<S> PartialEq for SchemeUpdateSet<S> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<S> Eq for SchemeUpdateSet<S> {}

impl<S> std::hash::Hash for SchemeUpdateSet<S> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
    }
}

/// Marker type for the [SharedMessageSender] of a prediction scheme.
pub struct UpdateSender<S>(PhantomData<S>);

/// Marker componet for clients that are part of a prediciton scheme.
#[derive(Component)]
pub struct PredictionClient<S>(PhantomData<S>);

impl<S> Default for PredictionClient<S> {
    fn default() -> Self {
        PredictionClient(PhantomData)
    }
}

fn send_update_syncs<S>(
    client_q: Query<Entity, With<PredictionClient<S>>>,
    time: Res<Time>,
    mut sender: SharedMessageSender<UpdateSender<S>>,
    message_id: Res<MessageId<WorldUpdateSync<S>>>,
) -> Result
where
    S: PredictionScheme,
{
    for client_entity in &client_q {
        sender.write(
            S::message_header(),
            client_entity,
            *message_id,
            true,
            &WorldUpdateSync::<S> {
                _p: PhantomData,
                update_time: time.elapsed(),
            },
        )?;
    }

    Ok(())
}
