use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::DeploymentArtifactIdentity;

use crate::attempt::{LoadAttempt, SharedAttemptResult};
use crate::{DeploymentLoadError, DeploymentOwnerConflict, DeploymentOwnerIdentity, LoadAttemptId};

pub(crate) struct CacheState<P, E> {
    slots: BTreeMap<DeploymentArtifactIdentity, OwnerSlot<P, E>>,
    last_attempt_id: u64,
}

struct OwnerSlot<P, E> {
    owner: DeploymentOwnerIdentity,
    entry: Option<CacheEntry<P, E>>,
}

enum CacheEntry<P, E> {
    Loading(Arc<LoadAttempt<P, E>>),
    Loaded(Arc<P>),
}

pub(crate) enum BeginLoad<P, E> {
    Loaded(Arc<P>),
    Join(Arc<LoadAttempt<P, E>>),
    Start(Arc<LoadAttempt<P, E>>),
}

impl<P, E> CacheState<P, E> {
    pub(crate) fn new() -> Self {
        Self {
            slots: BTreeMap::new(),
            last_attempt_id: 0,
        }
    }

    pub(crate) fn begin_load(
        &mut self,
        owner: DeploymentOwnerIdentity,
    ) -> Result<BeginLoad<P, E>, DeploymentLoadError<E>> {
        let build_id = owner.build_id().clone();
        if let Some(slot) = self.slots.get(&build_id) {
            if slot.owner != owner {
                return Err(DeploymentLoadError::OwnerConflict(
                    DeploymentOwnerConflict::new(build_id, slot.owner.clone(), owner),
                ));
            }
            match &slot.entry {
                Some(CacheEntry::Loaded(image)) => {
                    return Ok(BeginLoad::Loaded(Arc::clone(image)));
                }
                Some(CacheEntry::Loading(attempt)) => {
                    return Ok(BeginLoad::Join(Arc::clone(attempt)));
                }
                None => {}
            }
        }

        let attempt_id = self.next_attempt_id()?;
        let attempt = Arc::new(LoadAttempt::new(attempt_id, owner.clone()));
        match self.slots.get_mut(&build_id) {
            Some(slot) => slot.entry = Some(CacheEntry::Loading(Arc::clone(&attempt))),
            None => {
                self.slots.insert(
                    build_id,
                    OwnerSlot {
                        owner,
                        entry: Some(CacheEntry::Loading(Arc::clone(&attempt))),
                    },
                );
            }
        }
        Ok(BeginLoad::Start(attempt))
    }

    pub(crate) fn loaded(
        &self,
        owner: &DeploymentOwnerIdentity,
    ) -> Result<Option<Arc<P>>, DeploymentOwnerConflict> {
        let build_id = owner.build_id();
        let Some(slot) = self.slots.get(build_id) else {
            return Ok(None);
        };
        if &slot.owner != owner {
            return Err(DeploymentOwnerConflict::new(
                build_id.clone(),
                slot.owner.clone(),
                owner.clone(),
            ));
        }
        match &slot.entry {
            Some(CacheEntry::Loaded(image)) => Ok(Some(Arc::clone(image))),
            Some(CacheEntry::Loading(_)) | None => Ok(None),
        }
    }

    pub(crate) fn loaded_snapshot(&self) -> Box<[Arc<P>]> {
        self.slots
            .values()
            .filter_map(|slot| match &slot.entry {
                Some(CacheEntry::Loaded(image)) => Some(Arc::clone(image)),
                Some(CacheEntry::Loading(_)) | None => None,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice()
    }

    pub(crate) fn is_current(&self, attempt: &Arc<LoadAttempt<P, E>>) -> bool {
        self.slots
            .get(attempt.owner().build_id())
            .and_then(|slot| slot.entry.as_ref())
            .is_some_and(|entry| match entry {
                CacheEntry::Loading(current) => Arc::ptr_eq(current, attempt),
                CacheEntry::Loaded(_) => false,
            })
    }

    pub(crate) fn publish_attempt(
        &mut self,
        attempt: &Arc<LoadAttempt<P, E>>,
        result: &SharedAttemptResult<P, E>,
    ) {
        let Some(slot) = self.slots.get_mut(attempt.owner().build_id()) else {
            return;
        };
        let is_current = slot.entry.as_ref().is_some_and(|entry| match entry {
            CacheEntry::Loading(current) => Arc::ptr_eq(current, attempt),
            CacheEntry::Loaded(_) => false,
        });
        if !is_current {
            return;
        }
        slot.entry = match result {
            Ok(image) => Some(CacheEntry::Loaded(Arc::clone(image))),
            Err(_) => None,
        };
    }

    fn next_attempt_id(&mut self) -> Result<LoadAttemptId, DeploymentLoadError<E>> {
        let next = self
            .last_attempt_id
            .checked_add(1)
            .ok_or(DeploymentLoadError::AttemptIdExhausted)?;
        self.last_attempt_id = next;
        Ok(LoadAttemptId::new(next))
    }
}
