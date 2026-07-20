use serde::{Deserialize, Serialize};

use crate::RuntimeAssemblyRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssemblyActivationRejectReason {
    Resolve,
    Load,
    Link,
    Admission,
    ParticipantDisconnected,
}

/// Exact router <-> runtime control wire for whole-assembly activation.
///
/// Transition messages are intentionally assembly-scoped. Service ids, build
/// ids, artifact roots and executable targets cannot be represented here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AssemblyActivationControl {
    Prepare {
        environment: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
    },
    Prepared {
        environment: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
    },
    Reject {
        environment: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
        reason: AssemblyActivationRejectReason,
    },
    Commit {
        environment: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
    },
    Abort {
        environment: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
    },
    Register {
        environment: String,
        generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_language_golden_control_wire_decodes_strictly() {
        let fixture =
            include_str!("../../cross-system-fixtures/package-service-ecosystem/control-wire.json");
        let messages: Vec<AssemblyActivationControl> =
            serde_json::from_str(fixture).expect("canonical control fixture");
        assert_eq!(messages.len(), 6);

        let mut forbidden: serde_json::Value =
            serde_json::from_str(fixture).expect("fixture JSON value");
        for field in [
            "artifactRoots",
            "serviceConfig",
            "serviceId",
            "buildId",
            "target",
        ] {
            forbidden[0]
                .as_object_mut()
                .unwrap()
                .insert(field.to_string(), serde_json::json!("legacy"));
            assert!(
                serde_json::from_value::<Vec<AssemblyActivationControl>>(forbidden.clone())
                    .is_err(),
                "legacy field {field} must fail closed"
            );
            forbidden[0].as_object_mut().unwrap().remove(field);
        }
    }
}
