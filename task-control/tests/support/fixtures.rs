//! Canonical record builders for contract tests.

use skiff_artifact_model::{
    AssemblyIdentity, DeploymentArtifactIdentity, DeploymentRevision, PackageCallableId,
    RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef, ServiceDeploymentRef,
};
use skiff_task_control::model::{
    DetachedCallTarget, DurableUtcTimestamp, RecoverablePayload, ServiceOwner,
    TaskExecutionImageRef, TaskId, TaskRecord, TaskState, TaskTraceContext,
};

pub fn record(seed: u64, due_at_millis: i64) -> TaskRecord {
    TaskRecord {
        task_id: TaskId::new(format!("task-{seed}")),
        owner: ServiceOwner::new(format!("svc-{seed}")),
        execution: TaskExecutionImageRef {
            target_environment: "prod".to_string(),
            package_version: "1.0.0".to_string(),
            assembly: RuntimeAssemblyRef {
                assembly_identity: AssemblyIdentity::new(format!(
                    "skiff-runtime-assembly-v3:sha256:{seed:064x}"
                )),
            },
            config_snapshot: RuntimeConfigSnapshotRef {
                snapshot_id: RuntimeConfigSnapshotId::parse(format!(
                    "skiff-runtime-config-snapshot-v1:{seed:032x}"
                ))
                .expect("config id"),
            },
            deployment: ServiceDeploymentRef {
                service_id: format!("svc-{seed}"),
                contract_version: "1.0.0".to_string(),
                deployment_revision: DeploymentRevision::new("revision-1"),
                deployment_artifact_identity: DeploymentArtifactIdentity::new(
                    "deployment-identity",
                ),
            },
        },
        target: DetachedCallTarget::Function {
            callable: PackageCallableId::new(format!("callable-{seed}")),
        },
        payload: RecoverablePayload::new(vec![seed as u8, 2, 3]),
        due_at: DurableUtcTimestamp::from_millis(due_at_millis),
        state: TaskState::Scheduled,
        attempt_generation: 0,
        active_lease: None,
        terminal: None,
        trace: TaskTraceContext {
            trace_id: format!("trace-{seed}"),
            span_id: None,
        },
        created_at: DurableUtcTimestamp::from_millis(1),
        retry_not_before: None,
    }
}
