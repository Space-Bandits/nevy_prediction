//! The windowed client's camera, diagnostics overlay and workload hotkeys.
//!
//! - `[` / `]` lower and raise the cost of every tick in every client simulation world by 5ms.
//! - `\` resets the workload.

use std::time::Duration;

use bevy::dev_tools::diagnostics_overlay::{
    DiagnosticsOverlay, DiagnosticsOverlayItem, DiagnosticsOverlayPlugin,
    DiagnosticsOverlayStatistic,
};
use bevy::{diagnostic::DiagnosticPath, prelude::*};
use nevy_prediction::prelude::*;

use crate::{client::ClientWorkload, simulation::workload::Workload};

pub fn build(app: &mut App) {
    app.add_plugins((PredictionDiagnosticsPlugin, DiagnosticsOverlayPlugin));
    app.add_systems(Startup, (spawn_prediction_overlay, setup_camera));
    app.add_systems(Update, adjust_workload);
}

fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0., 15., 5.).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn adjust_workload(keyboard_input: Res<ButtonInput<KeyCode>>, mut workload: ResMut<ClientWorkload>) {
    const STEP: Duration = Duration::from_millis(5);

    let cost = workload.main.tick_cost;

    let new_cost = if keyboard_input.just_pressed(KeyCode::BracketRight) {
        cost + STEP
    } else if keyboard_input.just_pressed(KeyCode::BracketLeft) {
        cost.saturating_sub(STEP)
    } else if keyboard_input.just_pressed(KeyCode::Backslash) {
        Duration::ZERO
    } else {
        return;
    };

    info!("Client tick cost is now {:?}", new_cost);
    *workload = ClientWorkload::all(Workload::tick(new_cost));
}

fn spawn_prediction_overlay(mut commands: Commands) {
    let item = |path: DiagnosticPath, statistic, precision| DiagnosticsOverlayItem {
        path,
        statistic,
        precision,
    };

    use DiagnosticsOverlayStatistic::{Smoothed, Value};

    commands.spawn(DiagnosticsOverlay::new(
        "Prediction",
        vec![
            item(PredictionDiagnosticsPlugin::INTERVAL, Value, 1),
            item(PredictionDiagnosticsPlugin::TARGET, Value, 1),
            item(PredictionDiagnosticsPlugin::REQUIRED, Value, 1),
            item(PredictionDiagnosticsPlugin::RTT, Smoothed, 1),
            item(PredictionDiagnosticsPlugin::LATE_PROBES, Value, 0),
            item(PredictionDiagnosticsPlugin::LOST_PROBES, Value, 0),
            item(PredictionDiagnosticsPlugin::TIME_SCALE, Value, 2),
            item(PredictionDiagnosticsPlugin::CAPACITY, Smoothed, 2),
            item(PredictionDiagnosticsPlugin::MAIN_TICK, Value, 2),
            item(PredictionDiagnosticsPlugin::TEMPLATE_TICK, Value, 2),
            item(PredictionDiagnosticsPlugin::PREDICTION_TICK, Value, 2),
            item(PredictionDiagnosticsPlugin::PREDICTION_LENGTH, Value, 1),
            item(PredictionDiagnosticsPlugin::MAIN_LAG, Value, 0),
            item(PredictionDiagnosticsPlugin::TEMPLATE_LAG, Value, 0),
        ],
    ));
}
