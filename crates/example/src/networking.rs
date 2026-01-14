use bevy::prelude::*;
use nevy::prelude::*;

pub fn build(app: &mut App) {
    app.add_plugins(NevyPlugins::default());

    app.init_protocol::<()>();

    app.add_observer(log_status_changes);
}

fn log_status_changes(
    insert: On<Insert, ConnectionStatus>,
    status_q: Query<&ConnectionStatus>,
) -> Result {
    let status = status_q.get(insert.entity)?;
    info!(
        "Connection {} status changed to {:?}",
        insert.entity, status
    );
    Ok(())
}
