use std::collections::{BTreeMap, BTreeSet, VecDeque};

use skiff_runtime_linked_bytecode::FunctionIndex;

use crate::bytecode::{
    limits::LinkLimitTracker, BytecodeLinkError, BytecodeLinkLocation, LinkLimits,
};

/// Deterministic specialization intern table and FIFO worklist.
///
/// Roots are sorted by their complete specialization key. Targets discovered
/// while expanding one function are sorted by `(artifact_pc, key)`. A key is
/// assigned its dense function index when it is first enqueued, so direct and
/// mutual recursion reuse an already interned index rather than expanding a
/// second copy.
#[derive(Debug)]
pub(super) struct CanonicalWorklist<K> {
    pending: VecDeque<K>,
    indices: BTreeMap<K, FunctionIndex>,
}

impl<K> CanonicalWorklist<K>
where
    K: Clone + Ord,
{
    pub(super) fn try_from_roots(
        roots: impl IntoIterator<Item = K>,
        limits: &LinkLimits,
        location: BytecodeLinkLocation,
    ) -> Result<Self, BytecodeLinkError> {
        let roots = roots.into_iter().collect::<BTreeSet<_>>();
        let tracker = LinkLimitTracker::new(limits);
        tracker.check_root_specializations(roots.len() as u64, location.clone())?;
        tracker.check_specializations(roots.len() as u64, location.clone())?;

        let mut worklist = Self {
            pending: VecDeque::with_capacity(roots.len()),
            indices: BTreeMap::new(),
        };
        for root in roots {
            worklist.intern(root, limits, location.clone())?;
        }
        Ok(worklist)
    }

    pub(super) fn enqueue_discovered(
        &mut self,
        targets: impl IntoIterator<Item = (u32, K)>,
        limits: &LinkLimits,
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        for (_, target) in targets.into_iter().collect::<BTreeSet<_>>() {
            self.intern(target, limits, location.clone())?;
        }
        Ok(())
    }

    pub(super) fn pop_next(&mut self) -> Option<(FunctionIndex, K)> {
        let key = self.pending.pop_front()?;
        let index = self.indices[&key];
        Some((index, key))
    }

    pub(super) fn index_of(&self, key: &K) -> Option<FunctionIndex> {
        self.indices.get(key).copied()
    }

    pub(super) fn len(&self) -> usize {
        self.indices.len()
    }

    fn intern(
        &mut self,
        key: K,
        limits: &LinkLimits,
        location: BytecodeLinkLocation,
    ) -> Result<FunctionIndex, BytecodeLinkError> {
        if let Some(index) = self.indices.get(&key).copied() {
            return Ok(index);
        }

        let actual = (self.indices.len() as u64).checked_add(1).ok_or_else(|| {
            BytecodeLinkError::UnsatisfiedObligation {
                obligation: crate::bytecode::BytecodeLinkObligation::ConcreteSpecialization,
                location: location.clone(),
                detail: "arithmetic overflow while interning specialization".to_string(),
            }
        })?;
        LinkLimitTracker::new(limits).check_specializations(actual, location.clone())?;
        let raw_index =
            u32::try_from(self.indices.len()).map_err(|_| BytecodeLinkError::LimitExceeded {
                limit: crate::bytecode::BytecodeLinkLimit::Specializations,
                actual,
                max: u32::MAX as u64,
                location,
            })?;
        let index = FunctionIndex::new(raw_index);
        self.indices.insert(key.clone(), index);
        self.pending.push_back(key);
        Ok(index)
    }
}
