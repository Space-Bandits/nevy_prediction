use bevy::{
    ecs::{query::QueryIter, system::SystemParam},
    prelude::*,
};

use crate::state::JoinedClient;

#[derive(SystemParam)]
pub struct InitializePairs<'w, 's, C>
where
    C: Component,
{
    added_component_q: Query<'w, 's, Entity, Added<C>>,
    existing_component_q: Query<'w, 's, Entity, With<C>>,
    added_client_q: Query<'w, 's, Entity, Added<JoinedClient>>,
    existing_client_q: Query<'w, 's, Entity, With<JoinedClient>>,
}

impl<'w, 's, C> InitializePairs<'w, 's, C>
where
    C: Component,
{
    /// Returns pairs of (client, entity).
    pub fn iter(&self) -> impl Iterator<Item = (Entity, Entity)> + '_ {
        InitializePairsIter {
            queries: self,
            state: InitializePairsIterState::Components {
                iter: self.added_component_q.iter(),
                paired_iter_state: None,
            },
        }
    }
}

pub struct InitializePairsIter<'w, 's, 'b, C>
where
    C: Component,
    'b: 'w + 's,
{
    queries: &'b InitializePairs<'w, 's, C>,
    state: InitializePairsIterState<'w, 's, C>,
}

enum InitializePairsIterState<'w, 's, C>
where
    C: Component,
{
    /// Iterate over all added components for existing clients, except for clients added this tick.
    Components {
        /// Iterator for components.
        iter: QueryIter<'w, 's, Entity, Added<C>>,
        /// Iterator for clients with a component entity.
        paired_iter_state: Option<(Entity, QueryIter<'w, 's, Entity, With<JoinedClient>>)>,
    },
    /// Iterate over all added clients for existing components.
    Clients {
        /// Iterator for clients.
        iter: QueryIter<'w, 's, Entity, Added<JoinedClient>>,
        /// Iterator for components with a client entity.
        paired_iter_state: Option<(Entity, QueryIter<'w, 's, Entity, With<C>>)>,
    },
}

impl<'w, 's, 'b, C> Iterator for InitializePairsIter<'w, 's, 'b, C>
where
    C: Component,
    'b: 'w + 's,
{
    type Item = (Entity, Entity);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match &mut self.state {
                InitializePairsIterState::Components {
                    iter,
                    paired_iter_state,
                } => {
                    match paired_iter_state {
                        None => {
                            let Some(component_entity) = iter.next() else {
                                // advance to next iter state.

                                self.state = InitializePairsIterState::Clients {
                                    iter: self.queries.added_client_q.iter(),
                                    paired_iter_state: None,
                                };

                                continue;
                            };

                            *paired_iter_state =
                                Some((component_entity, self.queries.existing_client_q.iter()));

                            continue;
                        }
                        Some((component_entity, client_iter)) => {
                            let Some(client_entity) = client_iter.next() else {
                                *paired_iter_state = None;
                                continue;
                            };

                            // ignore clients that were added this tick to not double call
                            if self.queries.added_client_q.contains(client_entity) {
                                continue;
                            }

                            return Some((client_entity, *component_entity));
                        }
                    }
                }
                InitializePairsIterState::Clients {
                    iter,
                    paired_iter_state,
                } => {
                    match paired_iter_state {
                        None => {
                            let Some(client_entity) = iter.next() else {
                                // Iterator is finished
                                return None;
                            };

                            *paired_iter_state =
                                Some((client_entity, self.queries.existing_component_q.iter()));

                            continue;
                        }
                        Some((client_entity, component_iter)) => {
                            let Some(component_entity) = component_iter.next() else {
                                *paired_iter_state = None;
                                continue;
                            };

                            return Some((*client_entity, component_entity));
                        }
                    }
                }
            }
        }
    }
}
