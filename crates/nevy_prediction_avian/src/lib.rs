//! Runs [`avian3d`] physics in a [`nevy_prediction`] simulation.
//!
//! Add [`NevyPredictionAvianPlugin`] to your [`PredictionScheme`]'s plugin.
//! It is built into every world of the simulation and decides what to do in each one
//! from its [`SimulationInstance`]:
//!
//! - **Stepping worlds** (the server app's world and the client's main, template and prediction worlds)
//!   run the full [`PhysicsPlugins`] in [`SimulationUpdate`].
//!   On the server that means game logic can use [`SpatialQuery`] and read bodies directly.
//! - **The client app's world** never steps physics. After every extraction it updates the collider trees,
//!   so [`SpatialQuery`] and the hierarchy components like [`ColliderOf`] work against the latest state,
//!   writes [`Position`] and [`Rotation`] into [`Transform`],
//!   and interpolates bodies between the latest two ticks, see [`presentation`].
//! - **Snapshots** only carry the extracted state.
//!
//! # What crosses worlds
//!
//! On every [`SimulationEntity`]:
//!
//! - [`RigidBody`], which is inserted, replaced and removed to match the source world,
//! - [`Collider`], which only shares the source's shape so it is cheap,
//! - [`Position`], [`Rotation`], [`LinearVelocity`] and [`AngularVelocity`],
//! - every persistent force and acceleration, such as [`ConstantForce`],
//! - the computed mass properties, such as [`ComputedMass`],
//!
//! and the [`Time<Physics>`] resource.
//!
//! Everything else avian derives again in each world from these, such as
//! [`ColliderOf`] links, AABBs and the collider trees.
//! Colliders should be created with [`ColliderConstructor`], which is serializable,
//! so it can be sent in world updates and the [`Collider`] it builds is extracted.
//! Colliders that aren't on a simulation entity have to be built in every world that needs them.
//!
//! [`RigidBody`] and [`Collider`] require [`Transform`], because avian sizes a collider from its [`GlobalTransform`]
//! and bodies spawned by extraction only have what was extracted.
//!
//! Other components that affect physics, such as [`Mass`], damping or [`CollisionLayers`],
//! are left to the game. Either extract them with an
//! [`ExtractSimulationComponentPlugin`],
//! or derive them from extracted game state, for example with `#[require]`.
//!
//! # Correcting bodies
//!
//! Physics is not deterministic across worlds, see [solver state](#solver-state).
//! The server periodically sends the state of its bodies as [`UpdateRigidBody`] world updates,
//! which the client's template applies directly to [`Position`], [`Rotation`] and the velocities.
//! Add [`RigidBodySyncPlugin`](server::RigidBodySyncPlugin) to the server app to do this,
//! or send them yourself with [`UpdateRigidBody::from_body`].
//!
//! # Solver state
//!
//! Avian keeps state between steps that isn't extracted, most importantly the [`ContactGraph`],
//! whose contacts carry warm start impulses into the next step, and the constraint and island graphs.
//! All of it is keyed by local [`Entity`] and tied to avian's internal indices,
//! so it can't be copied into another world.
//!
//! This crate keeps each world's own solver state when another world is extracted into it.
//! When the prediction world forks from the template, its contacts are from the end of its previous prediction,
//! a few ticks ahead of the template's state. For resting and sliding contacts that is close to the template's own
//! warm start, which is closer than clearing them, which would give the prediction a cold start that the server never had.
//! Contacts that no longer exist are dropped by avian's narrow phase on the next step.
//!
//! So bodies in contact can simulate slightly differently on the server and every client world,
//! even with identical inputs, and the difference is bounded by [correcting bodies](#correcting-bodies).
//! Bodies that aren't touching anything simulate identically.
//! In the example crate's arena of 40 bouncing boxes the clients matched the server to within a millimeter at p99.
//! With 300 boxes the collisions are chaotic, and how often bodies are corrected decides how far they drift,
//! see [`RigidBodySyncSettings::interval`](server::RigidBodySyncSettings::interval).
//!
//! Avian is only deterministic across machines with its `enhanced-determinism` feature,
//! which is worth enabling when clients and the server run on different platforms.
//!
//! Island sleeping is disabled by default, because whether a body is asleep is also per world state,
//! and a body asleep in one world but awake in another diverges a lot.
//! [`IslandPlugin`] and [`IslandSleepingPlugin`] can be enabled again with [`NevyPredictionAvianPlugin::configure`].
//!
//! # Ordering
//!
//! World updates that spawn bodies or change their physics state should be applied before
//! [`PredictionPhysicsSystems::ApplyCorrections`],
//! so that a correction sent for the same tick as a spawn finds the body.
//!
//! [`SimulationUpdate`]: nevy_prediction::prelude::SimulationUpdate
//! [`SimulationEntity`]: nevy_prediction::prelude::SimulationEntity
//! [`SimulationInstance`]: nevy_prediction::prelude::SimulationInstance
//! [`ContactGraph`]: avian3d::prelude::ContactGraph
//! [`IslandPlugin`]: avian3d::dynamics::solver::islands::IslandPlugin
//! [`IslandSleepingPlugin`]: avian3d::dynamics::solver::islands::IslandSleepingPlugin

use avian3d::{
    dynamics::solver::islands::{IslandPlugin, IslandSleepingPlugin},
    prelude::*,
};
use bevy::{app::PluginGroupBuilder, prelude::*};
use nevy_prediction::prelude::*;

pub mod correction;
pub mod extract;
pub mod presentation;
pub mod server;

pub use correction::UpdateRigidBody;

pub mod prelude {
    pub use crate::{
        NevyPredictionAvianPlugin, PredictionPhysicsSystems,
        correction::UpdateRigidBody,
        presentation::{
            FollowSimulationBody, InterpolatedBody, PhysicsPresentationSystems,
            PresentationPhysics,
        },
        server::{NoRigidBodySync, RigidBodySyncPlugin, RigidBodySyncSettings},
    };
}

/// System sets in the [`SimulationUpdate`] of stepping worlds.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PredictionPhysicsSystems {
    /// Where [`UpdateRigidBody`] corrections are applied.
    ///
    /// Runs after [`DespawnSimulationEntities`]
    /// and before [`PhysicsSystems::First`].
    ApplyCorrections,
    /// Where [`ColliderConstructor`]s are turned into [`Collider`]s, after [`Self::ApplyCorrections`].
    ///
    /// avian does this in [`Update`], which simulation worlds don't run.
    /// Constructors that need a mesh aren't supported, and neither is [`ColliderConstructorHierarchy`].
    ///
    /// [`ColliderConstructorHierarchy`]: avian3d::prelude::ColliderConstructorHierarchy
    InitColliders,
}

/// Runs avian physics in every world of the simulation.
///
/// Add this to the plugin of your [`PredictionScheme`].
/// See the [crate] docs for what it does.
pub struct NevyPredictionAvianPlugin {
    /// See [`PhysicsPlugins::with_length_unit`].
    pub length_unit: f32,
    /// Changes the [`PhysicsPlugins`] that stepping worlds run.
    ///
    /// [`IslandPlugin`], [`IslandSleepingPlugin`], [`PhysicsInterpolationPlugin`] and [`ColliderCachePlugin`] are already disabled.
    /// Only change plugins that every stepping world should have,
    /// worlds that disagree about how physics works will mispredict.
    ///
    /// [`IslandPlugin`]: avian3d::dynamics::solver::islands::IslandPlugin
    /// [`IslandSleepingPlugin`]: avian3d::dynamics::solver::islands::IslandSleepingPlugin
    pub configure: fn(PluginGroupBuilder) -> PluginGroupBuilder,
}

impl Default for NevyPredictionAvianPlugin {
    fn default() -> Self {
        NevyPredictionAvianPlugin {
            length_unit: 1.,
            configure: |plugins| plugins,
        }
    }
}

impl NevyPredictionAvianPlugin {
    pub fn with_length_unit(mut self, length_unit: f32) -> Self {
        self.length_unit = length_unit;
        self
    }

    /// Sets [`Self::configure`].
    pub fn configure(mut self, configure: fn(PluginGroupBuilder) -> PluginGroupBuilder) -> Self {
        self.configure = configure;
        self
    }
}

impl Plugin for NevyPredictionAvianPlugin {
    fn build(&self, app: &mut App) {
        let instance = *app
            .world()
            .get_resource::<SimulationInstance>()
            .expect("`NevyPredictionAvianPlugin` should be added by a `PredictionScheme`'s plugin");

        // Simulation worlds are built from empty apps, but avian registers its types for reflection.
        app.init_resource::<AppTypeRegistry>();
        // Collider constructors can build colliders from meshes, so avian expects mesh assets to exist.
        // Simulations can't use meshes because they aren't extracted, but the resource still needs to exist.
        app.init_resource::<Assets<Mesh>>();

        extract::build(app);
        correction::build(app, instance);

        if instance.steps() || instance.is_app() {
            // avian sizes a collider from its `GlobalTransform` when it's inserted, and makes it zero sized without one.
            // Bodies spawned by extraction only have what was extracted, so make sure they have one.
            let _ = app.try_register_required_components::<RigidBody, Transform>();
            let _ = app.try_register_required_components::<Collider, Transform>();
        }

        if instance.steps() {
            let plugins = PhysicsPlugins::new(SimulationUpdate)
                .with_length_unit(self.length_unit)
                .build()
                .disable::<IslandSleepingPlugin>()
                .disable::<IslandPlugin>()
                .disable::<PhysicsInterpolationPlugin>()
                // simulation worlds don't have assets to build colliders from meshes with
                .disable::<ColliderCachePlugin>();

            app.add_plugins((self.configure)(plugins));

            app.configure_sets(
                SimulationUpdate,
                (
                    PredictionPhysicsSystems::ApplyCorrections,
                    PredictionPhysicsSystems::InitColliders,
                )
                    .chain()
                    .after(DespawnSimulationEntities)
                    .before(PhysicsSystems::First),
            );

            app.add_systems(
                SimulationUpdate,
                init_collider_constructors.in_set(PredictionPhysicsSystems::InitColliders),
            );
        } else if instance.is_app() {
            presentation::build(app, instance, self.length_unit);
        }
    }
}

fn init_collider_constructors(
    mut commands: Commands,
    constructor_q: Query<(Entity, &ColliderConstructor, Has<Collider>)>,
) {
    for (entity, constructor, has_collider) in &constructor_q {
        let mut entity_commands = commands.entity(entity);
        entity_commands.remove::<ColliderConstructor>();

        if has_collider {
            warn!("{entity} already had a collider when its `ColliderConstructor` was applied");
            continue;
        }

        match Collider::try_from_constructor(constructor.clone(), None) {
            Some(collider) => {
                entity_commands.insert(collider);
            }
            None => {
                error!("Couldn't build a collider for {entity} from {constructor:?}");
            }
        }
    }
}
