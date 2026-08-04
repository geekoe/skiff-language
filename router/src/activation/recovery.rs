//! Cold recovery projection for `ActivationCoordinator` (authority design
//! §4.2, C-activation-coordinator §4).
//!
//! On startup the coordinator reads durable state and distinguishes the two
//! explicit contracts: committed-only startup publishes the committed epoch,
//! while a durable pending installs a recovery transaction that rebinds new
//! exact sessions when expected replicas register (session epoch may change).
//! This module owns only the pure projection from durable state; the live
//! transaction machinery lives in [`super::coordinator`].

use skiff_artifact_model::{RuntimeAssemblyRef, RuntimeConfigSnapshotRef};
use skiff_deployment::activation_state::ProfileActivationState;

/// Typed refs used by the blocking loader to construct a whole `RoutingEpoch`
/// (committed epoch on recovery startup, candidate epoch for live or recovery
/// activation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateEpochRefs {
    pub profile: String,
    pub generation: u64,
    pub assembly: RuntimeAssemblyRef,
    pub config_snapshot: RuntimeConfigSnapshotRef,
}

impl CandidateEpochRefs {
    /// Durable→shared projection of the committed activation
    /// (C-bootstrap §2.2; total for a committed record).
    pub fn committed(state: &ProfileActivationState) -> Self {
        Self {
            profile: state.profile.clone(),
            generation: state.committed.generation,
            assembly: state.committed.assembly.clone(),
            config_snapshot: state.committed.config_snapshot.clone(),
        }
    }
}

/// A durable pending activation projected into coordinator terms. The
/// expected replica set comes from the frozen pending record; ephemeral
/// participant bindings are created later when replicas register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryTransaction {
    pub profile: String,
    pub activation_id: String,
    pub expected_generation: u64,
    pub candidate_generation: u64,
    pub assembly: RuntimeAssemblyRef,
    pub config_snapshot: RuntimeConfigSnapshotRef,
    pub expected_replica_ids: Vec<String>,
}

impl RecoveryTransaction {
    pub fn candidate_refs(&self) -> CandidateEpochRefs {
        CandidateEpochRefs {
            profile: self.profile.clone(),
            generation: self.candidate_generation,
            assembly: self.assembly.clone(),
            config_snapshot: self.config_snapshot.clone(),
        }
    }
}

/// Projects a recovery transaction from the durable pending record.
///
/// `None` means the profile has no pending activation: recovery reduces
/// to committed-only startup (publish the committed epoch and open the
/// listener; readiness is an E-session gate).
pub fn project_recovery(state: &ProfileActivationState) -> Option<RecoveryTransaction> {
    let pending = state.pending.as_ref()?;
    Some(RecoveryTransaction {
        profile: state.profile.clone(),
        activation_id: pending.activation_id.clone(),
        expected_generation: pending.expected_generation,
        candidate_generation: pending.candidate_generation,
        assembly: pending.assembly.clone(),
        config_snapshot: pending.config_snapshot.clone(),
        expected_replica_ids: pending.participant_replica_ids.clone(),
    })
}

/// Readiness gate projection for the recovery transaction
/// (C-activation-coordinator §4(7)): readiness opens only when every expected
/// replica has rebound to a new exact session. Pending recovery never blocks
/// the Runtime listener and is reported explicitly through health.
pub fn recovery_readiness(expected_replica_ids: &[String], waiting: usize) -> bool {
    expected_replica_ids.is_empty() || waiting == 0
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        AssemblyIdentity, RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
    };

    use super::*;

    fn assembly(byte: u8) -> RuntimeAssemblyRef {
        RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(format!(
                "skiff-runtime-assembly-v3:sha256:{}",
                char::from(b'a' + byte).to_string().repeat(64)
            )),
        }
    }

    fn config(byte: u8) -> RuntimeConfigSnapshotRef {
        RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(format!(
                "skiff-runtime-config-snapshot-v1:{}",
                char::from(b'a' + byte).to_string().repeat(32)
            ))
            .expect("config snapshot id"),
        }
    }

    fn state(pending: bool) -> ProfileActivationState {
        let mut state = ProfileActivationState::initial("test", 7, assembly(0), config(0));
        if pending {
            state.pending = Some(skiff_deployment::storage::PendingActivation {
                activation_id: "activation-8".to_string(),
                expected_generation: 7,
                candidate_generation: 8,
                assembly: assembly(1),
                config_snapshot: config(1),
                participant_replica_ids: vec!["runtime-a".to_string(), "runtime-b".to_string()],
            });
        }
        state
    }

    #[test]
    fn committed_refs_project_the_committed_tuple() {
        let refs = CandidateEpochRefs::committed(&state(false));
        assert_eq!(refs.profile, "test");
        assert_eq!(refs.generation, 7);
        assert_eq!(refs.assembly, assembly(0));
        assert_eq!(refs.config_snapshot, config(0));
    }

    #[test]
    fn project_recovery_is_none_without_pending() {
        assert_eq!(project_recovery(&state(false)), None);
    }

    #[test]
    fn project_recovery_carries_pending_refs_and_expected_replicas() {
        let recovery = project_recovery(&state(true)).expect("pending recovery");
        assert_eq!(recovery.activation_id, "activation-8");
        assert_eq!(recovery.expected_generation, 7);
        assert_eq!(recovery.candidate_generation, 8);
        assert_eq!(recovery.assembly, assembly(1));
        assert_eq!(recovery.config_snapshot, config(1));
        assert_eq!(
            recovery.expected_replica_ids,
            vec!["runtime-a".to_string(), "runtime-b".to_string()]
        );
        assert_eq!(
            recovery.candidate_refs(),
            CandidateEpochRefs {
                profile: "test".to_string(),
                generation: 8,
                assembly: assembly(1),
                config_snapshot: config(1),
            }
        );
    }

    #[test]
    fn readiness_requires_zero_waiting_after_rebind() {
        assert!(!recovery_readiness(&["runtime-a".to_string()], 1));
        assert!(recovery_readiness(&["runtime-a".to_string()], 0));
    }
}
