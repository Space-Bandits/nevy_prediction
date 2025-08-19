use bevy::{
    ecs::{query::QueryIter, system::SystemParam},
    prelude::*,
};

#[derive(SystemParam)]
pub struct NewPairs<'w, 's, A, B>
where
    A: Component,
    B: Component,
{
    added_a: Query<'w, 's, Entity, Added<A>>,
    existing_a: Query<'w, 's, Entity, With<A>>,
    added_b: Query<'w, 's, Entity, Added<B>>,
    existing_b: Query<'w, 's, Entity, With<B>>,
}

impl<'w, 's, A, B> NewPairs<'w, 's, A, B>
where
    A: Component,
    B: Component,
{
    /// Returns pairs of entities with `A` and `B` respectively, where either or both entities had that component added this tick.
    /// If both `A` and `B` were added this tick they will not be double counted.
    /// If both `A` and `B` are on the same entity this will be included.
    pub fn iter<'b>(&'b self) -> InitializePairsIter<'w, 's, 'b, A, B> {
        InitializePairsIter {
            queries: self,
            state: InitializePairsIterState::NewA {
                iter: self.added_a.iter(),
                paired_iter_state: None,
            },
        }
    }
}

impl<'w, 's, 'b, A, B> IntoIterator for &'b NewPairs<'w, 's, A, B>
where
    A: Component,
    B: Component,
    'b: 'w + 's,
{
    type IntoIter = InitializePairsIter<'w, 's, 'b, A, B>;
    type Item = (Entity, Entity);

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct InitializePairsIter<'w, 's, 'b, A, B>
where
    A: Component,
    B: Component,
    'b: 'w + 's,
{
    queries: &'b NewPairs<'w, 's, A, B>,
    state: InitializePairsIterState<'w, 's, A, B>,
}

enum InitializePairsIterState<'w, 's, A, B>
where
    A: Component,
    B: Component,
{
    /// Iterate over all added `A` for existing `B`, except for `B` added this tick.
    NewA {
        /// Iterator for added `A`.
        iter: QueryIter<'w, 's, Entity, Added<A>>,
        /// Iterator for existing `B`.
        paired_iter_state: Option<(Entity, QueryIter<'w, 's, Entity, With<B>>)>,
    },
    /// Iterate over all added `B` for existing `A`.
    NewB {
        /// Iterator for added `B`
        iter: QueryIter<'w, 's, Entity, Added<B>>,
        /// Iterator for existing `A`.
        paired_iter_state: Option<(Entity, QueryIter<'w, 's, Entity, With<A>>)>,
    },
}

impl<'w, 's, 'b, A, B> Iterator for InitializePairsIter<'w, 's, 'b, A, B>
where
    A: Component,
    B: Component,
    'b: 'w + 's,
{
    type Item = (Entity, Entity);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match &mut self.state {
                InitializePairsIterState::NewA {
                    iter,
                    paired_iter_state,
                } => {
                    match paired_iter_state {
                        None => {
                            let Some(added_a_entity) = iter.next() else {
                                // Reached end of all added `A`, advance to next state.

                                self.state = InitializePairsIterState::NewB {
                                    iter: self.queries.added_b.iter(),
                                    paired_iter_state: None,
                                };

                                continue;
                            };

                            *paired_iter_state =
                                Some((added_a_entity, self.queries.existing_b.iter()));

                            continue;
                        }
                        Some((added_a_entity, existing_b_iter)) => {
                            let Some(existing_b_entity) = existing_b_iter.next() else {
                                *paired_iter_state = None;
                                continue;
                            };

                            // ignore `B` that were added this tick to not double count.
                            if self.queries.added_b.contains(existing_b_entity) {
                                continue;
                            }

                            return Some((*added_a_entity, existing_b_entity));
                        }
                    }
                }
                InitializePairsIterState::NewB {
                    iter,
                    paired_iter_state,
                } => {
                    match paired_iter_state {
                        None => {
                            let Some(added_b_entity) = iter.next() else {
                                // Iterator is finished
                                return None;
                            };

                            *paired_iter_state =
                                Some((added_b_entity, self.queries.existing_a.iter()));

                            continue;
                        }
                        Some((added_b_entity, component_iter)) => {
                            let Some(existing_a_entity) = component_iter.next() else {
                                *paired_iter_state = None;
                                continue;
                            };

                            return Some((existing_a_entity, *added_b_entity));
                        }
                    }
                }
            }
        }
    }
}
