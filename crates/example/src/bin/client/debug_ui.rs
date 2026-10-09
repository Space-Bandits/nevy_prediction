use bevy::dev_tools::diagnostics_overlay::{
    DiagnosticsOverlay, DiagnosticsOverlayItem, DiagnosticsOverlayPlugin,
    DiagnosticsOverlayStatistic,
};
use bevy::{diagnostic::DiagnosticPath, prelude::*};
use nevy_prediction::prelude::*;

pub fn build(app: &mut App) {
    app.add_plugins((PredictionDiagnosticsPlugin, DiagnosticsOverlayPlugin));
    app.add_systems(Startup, spawn_prediction_overlay);
}

fn spawn_prediction_overlay(mut commands: Commands) {
    let item = |path: DiagnosticPath, statistic, precision| DiagnosticsOverlayItem {
        path,
        statistic,
        precision,
    };

    commands.spawn(DiagnosticsOverlay::new(
        "Prediction",
        vec![
            item(
                PredictionDiagnosticsPlugin::INTERVAL,
                DiagnosticsOverlayStatistic::Value,
                1,
            ),
            item(
                PredictionDiagnosticsPlugin::TARGET,
                DiagnosticsOverlayStatistic::Value,
                1,
            ),
            item(
                PredictionDiagnosticsPlugin::REQUIRED,
                DiagnosticsOverlayStatistic::Value,
                1,
            ),
            item(
                PredictionDiagnosticsPlugin::RTT,
                DiagnosticsOverlayStatistic::Smoothed,
                1,
            ),
            item(
                PredictionDiagnosticsPlugin::LATE_PROBES,
                DiagnosticsOverlayStatistic::Value,
                0,
            ),
            item(
                PredictionDiagnosticsPlugin::LOST_PROBES,
                DiagnosticsOverlayStatistic::Value,
                0,
            ),
        ],
    ));
}
