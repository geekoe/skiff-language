//! Deployment-internal wire boundary between Router and its trusted activation
//! state backend.
//!
//! This module is intentionally not part of `skiff-trusted-registry-contract`:
//! prepare/commit/abort are coordinator implementation details, while the public
//! registry surface exposes only atomic activation.

use std::{
    collections::BTreeSet,
    future::Future,
    io::{BufRead, Read, Write},
    pin::Pin,
};

use serde::{de, Deserialize, Deserializer, Serialize};
use skiff_artifact_identity::service_contract_ref;
use skiff_artifact_model::{
    validate_activation_token, RuntimeAssembly, RuntimeAssemblyRef, ServiceContract,
    ServiceContractRef,
};

pub const MAX_BACKEND_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationBackendRef {
    pub environment: String,
    pub activation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationBackendCommit {
    pub environment: String,
    pub activation_id: String,
    #[serde(deserialize_with = "deserialize_replica_ids")]
    pub connected_replica_ids: Vec<String>,
    #[serde(deserialize_with = "deserialize_replica_ids")]
    pub prepared_replica_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationBackendPrepare {
    pub environment: String,
    pub activation_id: String,
    pub expected_generation: u64,
    pub candidate_generation: u64,
    pub assembly: RuntimeAssemblyRef,
    pub participant_replica_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationBackendCommitted {
    pub generation: u64,
    pub assembly: RuntimeAssemblyRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationBackendPending {
    pub activation_id: String,
    pub expected_generation: u64,
    pub candidate_generation: u64,
    pub assembly: RuntimeAssemblyRef,
    pub participant_replica_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationBackendSnapshot {
    pub environment: String,
    pub committed: Option<ActivationBackendCommitted>,
    pub pending: Option<ActivationBackendPending>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadActivation {
    pub environment: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadAssembly {
    pub assembly: RuntimeAssemblyRef,
}

/// Backend-neutral Router projection. Ingress modes remain exact immutable
/// ServiceContract facts rather than being reconstructed from assembly data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouterBackendSnapshot {
    pub assembly: RuntimeAssembly,
    pub service_contracts: Vec<ServiceContract>,
}

impl RouterBackendSnapshot {
    pub fn from_canonical(
        assembly: RuntimeAssembly,
        service_contracts: Vec<ServiceContract>,
    ) -> Result<Self, ActivationBackendError> {
        let expected = assembly
            .resolved_contracts
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut actual = BTreeSet::<ServiceContractRef>::new();
        for contract in &service_contracts {
            let reference =
                service_contract_ref(contract).map_err(|error| ActivationBackendError {
                    code: "invalid-service-contract".into(),
                    message: error.to_string(),
                })?;
            if !actual.insert(reference) {
                return Err(ActivationBackendError {
                    code: "duplicate-service-contract".into(),
                    message: "router snapshot contains a duplicate exact ServiceContract".into(),
                });
            }
        }
        if actual != expected {
            return Err(ActivationBackendError {
                code: "service-contract-set-mismatch".into(),
                message:
                    "router snapshot ServiceContracts must exactly match assembly.resolvedContracts"
                        .into(),
            });
        }
        Ok(Self {
            assembly,
            service_contracts,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "operation",
    content = "payload",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum ActivationBackendOperation {
    Read(ReadActivation),
    ReadSnapshot(ReadAssembly),
    Prepare(ActivationBackendPrepare),
    Commit(ActivationBackendCommit),
    Abort(ActivationBackendRef),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationBackendRequest {
    pub request_id: String,
    #[serde(flatten)]
    pub operation: ActivationBackendOperation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationBackendError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ActivationBackendOutcome {
    RouterSnapshot { snapshot: RouterBackendSnapshot },
    Success { snapshot: ActivationBackendSnapshot },
    Failure { error: ActivationBackendError },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationBackendResponse {
    pub request_id: String,
    #[serde(flatten)]
    pub outcome: ActivationBackendOutcome,
}

pub type ActivationBackendFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ActivationBackendError>> + Send + 'a>>;

/// Internal durable activation state boundary. Implementations derive and
/// persist audit entries in the same transaction as state; callers cannot
/// supply audit payloads.
pub trait RouterActivationBackend: Send + Sync {
    fn read(
        &self,
        request: ReadActivation,
    ) -> ActivationBackendFuture<'_, ActivationBackendSnapshot>;
    fn read_snapshot(
        &self,
        request: ReadAssembly,
    ) -> ActivationBackendFuture<'_, RouterBackendSnapshot>;
    fn prepare(
        &self,
        request: ActivationBackendPrepare,
    ) -> ActivationBackendFuture<'_, ActivationBackendSnapshot>;
    fn commit(
        &self,
        request: ActivationBackendCommit,
    ) -> ActivationBackendFuture<'_, ActivationBackendSnapshot>;
    fn abort(
        &self,
        request: ActivationBackendRef,
    ) -> ActivationBackendFuture<'_, ActivationBackendSnapshot>;
}

#[derive(Debug, thiserror::Error)]
pub enum ActivationBackendEnvelopeError {
    #[error("activation backend frame exceeds {MAX_BACKEND_FRAME_BYTES} bytes")]
    FrameTooLarge,
    #[error("activation backend request is invalid: {0}")]
    InvalidRequest(#[from] serde_json::Error),
    #[error("activation backend transport failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Serves a long-lived, sequential NDJSON connection. Sequential dispatch is
/// deliberate: it bounds in-flight memory and preserves backend operation
/// ordering. EOF cleanly ends the adapter process.
pub async fn serve_backend_ndjson<B, R, W>(
    backend: &B,
    mut input: R,
    mut output: W,
) -> Result<(), ActivationBackendEnvelopeError>
where
    B: RouterActivationBackend,
    R: BufRead,
    W: Write,
{
    let mut frame = Vec::new();
    loop {
        frame.clear();
        let read = Read::take(&mut input, (MAX_BACKEND_FRAME_BYTES + 2) as u64)
            .read_until(b'\n', &mut frame)?;
        if read == 0 {
            return Ok(());
        }
        if frame.len() > MAX_BACKEND_FRAME_BYTES + 1
            || (frame.len() == MAX_BACKEND_FRAME_BYTES + 1 && frame.last() != Some(&b'\n'))
        {
            return Err(ActivationBackendEnvelopeError::FrameTooLarge);
        }
        if frame.last() == Some(&b'\n') {
            frame.pop();
        }
        if frame.last() == Some(&b'\r') {
            frame.pop();
        }
        if frame.is_empty() || frame.len() > MAX_BACKEND_FRAME_BYTES {
            return Err(if frame.is_empty() {
                ActivationBackendEnvelopeError::InvalidRequest(
                    serde_json::from_slice::<ActivationBackendRequest>(&frame).unwrap_err(),
                )
            } else {
                ActivationBackendEnvelopeError::FrameTooLarge
            });
        }
        let request: ActivationBackendRequest = serde_json::from_slice(&frame)?;
        validate_request_id(&request.request_id)?;
        let request_id = request.request_id;
        let result = match request.operation {
            ActivationBackendOperation::Read(value) => backend
                .read(value)
                .await
                .map(|snapshot| ActivationBackendOutcome::Success { snapshot }),
            ActivationBackendOperation::ReadSnapshot(value) => backend
                .read_snapshot(value)
                .await
                .map(|snapshot| ActivationBackendOutcome::RouterSnapshot { snapshot }),
            ActivationBackendOperation::Prepare(value) => backend
                .prepare(value)
                .await
                .map(|snapshot| ActivationBackendOutcome::Success { snapshot }),
            ActivationBackendOperation::Commit(value) => backend
                .commit(value)
                .await
                .map(|snapshot| ActivationBackendOutcome::Success { snapshot }),
            ActivationBackendOperation::Abort(value) => backend
                .abort(value)
                .await
                .map(|snapshot| ActivationBackendOutcome::Success { snapshot }),
        };
        let response = ActivationBackendResponse {
            request_id,
            outcome: match result {
                Ok(outcome) => outcome,
                Err(error) => ActivationBackendOutcome::Failure { error },
            },
        };
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}

fn validate_request_id(request_id: &str) -> Result<(), ActivationBackendEnvelopeError> {
    if request_id.is_empty()
        || request_id.len() > 128
        || !request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        let source = serde_json::from_str::<ActivationBackendRequest>("{}").unwrap_err();
        return Err(ActivationBackendEnvelopeError::InvalidRequest(source));
    }
    Ok(())
}

fn deserialize_replica_ids<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<String>::deserialize(deserializer)?;
    let mut unique = BTreeSet::new();
    for value in &values {
        validate_activation_token(value, "replicaId").map_err(de::Error::custom)?;
        if !unique.insert(value) {
            return Err(de::Error::custom(format!(
                "replicaIds contains duplicate {value}"
            )));
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use std::{
        io::Cursor,
        sync::{Arc, Mutex},
    };

    use super::*;

    #[derive(Default)]
    struct RecordingBackend {
        operations: Arc<Mutex<Vec<&'static str>>>,
    }

    impl RecordingBackend {
        fn result(
            &self,
            operation: &'static str,
            environment: String,
        ) -> ActivationBackendFuture<'_, ActivationBackendSnapshot> {
            self.operations.lock().unwrap().push(operation);
            Box::pin(async move {
                Ok(ActivationBackendSnapshot {
                    environment,
                    committed: None,
                    pending: None,
                })
            })
        }
    }

    impl RouterActivationBackend for RecordingBackend {
        fn read(
            &self,
            request: ReadActivation,
        ) -> ActivationBackendFuture<'_, ActivationBackendSnapshot> {
            self.result("read", request.environment)
        }
        fn read_snapshot(
            &self,
            _request: ReadAssembly,
        ) -> ActivationBackendFuture<'_, RouterBackendSnapshot> {
            self.operations.lock().unwrap().push("read-snapshot");
            Box::pin(async {
                Err(ActivationBackendError {
                    code: "not-found".into(),
                    message: "fixture".into(),
                })
            })
        }
        fn prepare(
            &self,
            request: ActivationBackendPrepare,
        ) -> ActivationBackendFuture<'_, ActivationBackendSnapshot> {
            self.result("prepare", request.environment)
        }
        fn commit(
            &self,
            request: ActivationBackendCommit,
        ) -> ActivationBackendFuture<'_, ActivationBackendSnapshot> {
            self.result("commit", request.environment)
        }
        fn abort(
            &self,
            request: ActivationBackendRef,
        ) -> ActivationBackendFuture<'_, ActivationBackendSnapshot> {
            self.result("abort", request.environment)
        }
    }

    #[tokio::test]
    async fn dispatches_typed_frames_and_correlates_request_ids() {
        let input =
            br#"{"requestId":"one","operation":"read","payload":{"environment":"prod"}}
{"requestId":"two","operation":"commit","payload":{"environment":"prod","activationId":"a","connectedReplicaIds":["r1","r2"],"preparedReplicaIds":["r2","r1"]}}
"#;
        let backend = RecordingBackend::default();
        let mut output = Vec::new();
        serve_backend_ndjson(&backend, Cursor::new(input), &mut output)
            .await
            .unwrap();
        let lines = String::from_utf8(output).unwrap();
        assert!(lines.contains(r#""requestId":"one""#));
        assert!(lines.contains(r#""requestId":"two""#));
        assert_eq!(*backend.operations.lock().unwrap(), ["read", "commit"]);
    }

    #[test]
    fn rejects_router_supplied_audit_payload_and_unknown_operations() {
        let audit = serde_json::json!({
            "requestId": "one",
            "operation": "commit",
            "payload": {
                "environment": "prod",
                "activationId": "a",
                "connectedReplicaIds": ["r1"],
                "preparedReplicaIds": ["r1"],
                "audit": {"status": "committed"}
            }
        });
        assert!(serde_json::from_value::<ActivationBackendRequest>(audit).is_err());
        let unknown = serde_json::json!({
            "requestId": "one",
            "operation": "activate",
            "payload": {"environment": "prod"}
        });
        assert!(serde_json::from_value::<ActivationBackendRequest>(unknown).is_err());
    }

    #[test]
    fn commit_replica_collections_are_exact_and_deduplicated() {
        let valid = serde_json::json!({
            "requestId": "commit-1",
            "operation": "commit",
            "payload": {
                "environment": "prod",
                "activationId": "a",
                "connectedReplicaIds": ["r2", "r1"],
                "preparedReplicaIds": ["r1"]
            }
        });
        let decoded = serde_json::from_value::<ActivationBackendRequest>(valid).unwrap();
        let ActivationBackendOperation::Commit(commit) = decoded.operation else {
            panic!("expected commit");
        };
        assert_eq!(commit.connected_replica_ids, ["r2", "r1"]);
        assert_eq!(commit.prepared_replica_ids, ["r1"]);

        for duplicate_field in ["connectedReplicaIds", "preparedReplicaIds"] {
            let mut invalid = serde_json::json!({
                "requestId": "commit-2",
                "operation": "commit",
                "payload": {
                    "environment": "prod",
                    "activationId": "a",
                    "connectedReplicaIds": ["r1"],
                    "preparedReplicaIds": ["r1"]
                }
            });
            invalid["payload"][duplicate_field] = serde_json::json!(["r1", "r1"]);
            assert!(
                serde_json::from_value::<ActivationBackendRequest>(invalid).is_err(),
                "{duplicate_field} duplicates must fail closed"
            );
        }

        let missing = serde_json::json!({
            "requestId": "commit-3",
            "operation": "commit",
            "payload": {
                "environment": "prod",
                "activationId": "a",
                "connectedReplicaIds": ["r1"]
            }
        });
        assert!(serde_json::from_value::<ActivationBackendRequest>(missing).is_err());
    }

    #[test]
    fn router_snapshot_wire_and_contract_projection_fail_closed() {
        let assembly = crate::fixtures::empty_runtime_assembly_fixture().unwrap();
        let snapshot = RouterBackendSnapshot::from_canonical(assembly.clone(), Vec::new()).unwrap();
        let encoded = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(encoded["serviceContracts"], serde_json::json!([]));
        assert_eq!(
            serde_json::from_value::<RouterBackendSnapshot>(encoded.clone()).unwrap(),
            snapshot
        );

        let mut missing = encoded.clone();
        missing.as_object_mut().unwrap().remove("serviceContracts");
        assert!(serde_json::from_value::<RouterBackendSnapshot>(missing).is_err());
        let mut unknown = encoded;
        unknown["operationModes"] = serde_json::json!({});
        assert!(serde_json::from_value::<RouterBackendSnapshot>(unknown).is_err());

        let mut requires_contract = assembly;
        requires_contract
            .resolved_contracts
            .push(crate::fixtures::service_contract_ref_fixture());
        let error =
            RouterBackendSnapshot::from_canonical(requires_contract, Vec::new()).unwrap_err();
        assert_eq!(error.code, "service-contract-set-mismatch");
    }

    #[tokio::test]
    async fn rejects_oversized_frames_before_dispatch() {
        let backend = RecordingBackend::default();
        let input = vec![b'x'; MAX_BACKEND_FRAME_BYTES + 2];
        let error = serve_backend_ndjson(&backend, Cursor::new(input), Vec::new())
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ActivationBackendEnvelopeError::FrameTooLarge
        ));
        assert!(backend.operations.lock().unwrap().is_empty());
    }
}
