use std::marker::PhantomData;

use bevy::prelude::*;

use crate::client::parallel_app::{ExtractSimulation, SourceWorld};

/// This plugin acts as a utility to automatically extract a resource.
pub struct ExtractSimulationResourcePlugin<R>(PhantomData<R>);

impl<R> Default for ExtractSimulationResourcePlugin<R> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<R> Plugin for ExtractSimulationResourcePlugin<R>
where
    R: Send + Sync + 'static + Resource + Clone,
{
    fn build(&self, app: &mut App) {
        app.add_systems(ExtractSimulation, extract_resource::<R>);
    }
}

fn extract_resource<R>(source_world: Res<SourceWorld>, local_resource: Option<ResMut<R>>) -> Result
where
    R: Resource + Clone,
{
    let source_resource = source_world.get_resource::<R>().ok_or(format!(
        "Resource {} was not present in source world when trying to extract it",
        std::any::type_name::<R>(),
    ))?;

    let mut local_resource = local_resource.ok_or(format!(
        "Resource {} was not present in local world when trying to extract it",
        std::any::type_name::<R>(),
    ))?;

    *local_resource = source_resource.clone();

    Ok(())
}
