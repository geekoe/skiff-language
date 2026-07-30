use std::sync::Arc;

use skiff_artifact_model::{
    AssemblyActivationControl, AssemblyActivationRejectReason, AssemblyIdentity,
    CanonicalPackageLinkPlan, FileIrRef, FileIrUnit, PackageArtifact, PackageArtifactRef,
    PublicationResourceRef, RuntimeAssembly, RuntimeAssemblyRef, RuntimeConfigSnapshotRef,
    ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_runtime_host::{DbProviderSource, RuntimeConfig, RuntimeHost};
use skiff_runtime_loader::{RuntimeAssemblyContentResolver, RuntimeAssemblyRecordResolver};

#[derive(Debug)]
struct SnapshotResolveError;

impl std::fmt::Display for SnapshotResolveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("snapshot ref mismatch")
    }
}

impl std::error::Error for SnapshotResolveError {}

#[derive(Clone)]
struct EmptySnapshotResolver {
    snapshot: skiff_runtime_config_snapshot::RuntimeConfigSnapshot,
}

impl skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver for EmptySnapshotResolver {
    type Error = SnapshotResolveError;

    fn resolve(
        &self,
        reference: &RuntimeConfigSnapshotRef,
    ) -> Result<skiff_runtime_config_snapshot::RuntimeConfigSnapshot, Self::Error> {
        (self.snapshot.snapshot_ref() == reference)
            .then(|| self.snapshot.clone())
            .ok_or(SnapshotResolveError)
    }
}

fn empty_snapshot(environment: &str) -> (RuntimeConfigSnapshotRef, EmptySnapshotResolver) {
    let reference = skiff_runtime_config_snapshot::new_runtime_config_snapshot_ref();
    let snapshot = skiff_runtime_config_snapshot::RuntimeConfigSnapshot::new(
        environment,
        reference.clone(),
        Vec::new(),
    )
    .unwrap();
    (reference, EmptySnapshotResolver { snapshot })
}

struct EmptyAssemblyResolver {
    assembly: Arc<RuntimeAssembly>,
}

impl RuntimeAssemblyRecordResolver for EmptyAssemblyResolver {
    fn resolve_runtime_assembly(
        &self,
        _reference: &RuntimeAssemblyRef,
    ) -> anyhow::Result<Arc<RuntimeAssembly>> {
        Ok(Arc::clone(&self.assembly))
    }
}

impl RuntimeAssemblyContentResolver for EmptyAssemblyResolver {
    fn resolve_deployment(
        &self,
        _reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        anyhow::bail!("empty assembly must not resolve deployments")
    }

    fn resolve_contract(
        &self,
        _reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        anyhow::bail!("empty assembly must not resolve contracts")
    }

    fn resolve_package_schema_type(
        &self,
        _reference: &skiff_artifact_model::PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::PackageSchemaTypeRecord>> {
        anyhow::bail!("empty assembly must not resolve package schema")
    }

    fn resolve_package(
        &self,
        _reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        anyhow::bail!("empty assembly must not resolve packages")
    }

    fn resolve_file_ir(
        &self,
        _package: &PackageArtifactRef,
        _reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        anyhow::bail!("empty assembly must not resolve File IR")
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        _reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        anyhow::bail!("empty assembly must not resolve resources")
    }
}

fn empty_assembly() -> RuntimeAssembly {
    let mut assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("unassigned"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut assembly).unwrap();
    assembly
}

fn runtime_host(replica_id: &str) -> RuntimeHost {
    RuntimeHost::new(RuntimeConfig {
        db_provider: DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:1/runtime".to_string(),
        base_runtime_id: replica_id.to_string(),
        runtime_home: std::env::temp_dir().join(replica_id),
        environment: "prod".to_string(),
        http_response_max_bytes: skiff_runtime_host::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
    })
    .unwrap()
}

fn transition(
    kind: &str,
    reference: RuntimeAssemblyRef,
    config_snapshot: RuntimeConfigSnapshotRef,
    replica_id: &str,
) -> AssemblyActivationControl {
    let fields = (
        "prod".to_string(),
        "activation-1".to_string(),
        0,
        1,
        reference,
        config_snapshot,
        replica_id.to_string(),
    );
    match (kind, fields) {
        (
            "prepare",
            (
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
            ),
        ) => AssemblyActivationControl::Prepare {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
            service_db: None,
        },
        (
            "commit",
            (
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
            ),
        ) => AssemblyActivationControl::Commit {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
            service_db: None,
        },
        (
            "abort",
            (
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
            ),
        ) => AssemblyActivationControl::Abort {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
        },
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn prepare_abort_commit_replay_and_cold_recovery_are_atomic() {
    let assembly = Arc::new(empty_assembly());
    let reference = skiff_artifact_identity::runtime_assembly_ref(&assembly).unwrap();
    let resolver = EmptyAssemblyResolver { assembly };
    let (config_snapshot, config_resolver) = empty_snapshot("prod");
    let host = runtime_host("runtime-a");

    let prepared = host
        .apply_assembly_activation_control(
            transition(
                "prepare",
                reference.clone(),
                config_snapshot.clone(),
                "runtime-a",
            ),
            &resolver,
            &config_resolver,
        )
        .await
        .unwrap();
    assert!(matches!(
        prepared,
        Some(AssemblyActivationControl::Prepared { .. })
    ));
    assert!(host.active_assembly_registration().unwrap().is_none());

    host.apply_assembly_activation_control(
        transition(
            "abort",
            reference.clone(),
            config_snapshot.clone(),
            "runtime-a",
        ),
        &resolver,
        &config_resolver,
    )
    .await
    .unwrap();
    assert!(host.active_assembly_registration().unwrap().is_none());

    host.apply_assembly_activation_control(
        transition(
            "prepare",
            reference.clone(),
            config_snapshot.clone(),
            "runtime-a",
        ),
        &resolver,
        &config_resolver,
    )
    .await
    .unwrap();
    let committed = host
        .apply_assembly_activation_control(
            transition(
                "commit",
                reference.clone(),
                config_snapshot.clone(),
                "runtime-a",
            ),
            &resolver,
            &config_resolver,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        committed,
        AssemblyActivationControl::Register { generation: 1, .. }
    ));

    let replay = host
        .apply_assembly_activation_control(
            transition(
                "commit",
                reference.clone(),
                config_snapshot.clone(),
                "runtime-a",
            ),
            &resolver,
            &config_resolver,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(replay, committed);

    let restarted = runtime_host("runtime-a");
    let recovered = restarted
        .apply_assembly_activation_control(
            transition("commit", reference, config_snapshot, "runtime-a"),
            &resolver,
            &config_resolver,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered, committed);
}

#[tokio::test]
async fn prepare_rejects_dev_config_snapshot_for_prod_before_ack() {
    let assembly = Arc::new(empty_assembly());
    let reference = skiff_artifact_identity::runtime_assembly_ref(&assembly).unwrap();
    let resolver = EmptyAssemblyResolver { assembly };
    let (config_snapshot, config_resolver) = empty_snapshot("dev");
    let host = runtime_host("runtime-a");

    let reply = host
        .apply_assembly_activation_control(
            transition("prepare", reference, config_snapshot, "runtime-a"),
            &resolver,
            &config_resolver,
        )
        .await
        .expect("environment mismatch is a fail-closed activation reply")
        .expect("prepare must receive a reply");

    assert!(matches!(
        reply,
        AssemblyActivationControl::Reject {
            reason: AssemblyActivationRejectReason::Admission,
            ..
        }
    ));
    assert!(host.active_assembly_registration().unwrap().is_none());
}

#[tokio::test]
async fn rejected_exact_ref_preserves_committed_generation_and_two_replicas_are_independent() {
    let assembly = Arc::new(empty_assembly());
    let reference = skiff_artifact_identity::runtime_assembly_ref(&assembly).unwrap();
    let resolver = EmptyAssemblyResolver { assembly };
    let (config_snapshot, config_resolver) = empty_snapshot();
    let first = runtime_host("runtime-a");
    let second = runtime_host("runtime-b");

    for (host, replica) in [(&first, "runtime-a"), (&second, "runtime-b")] {
        host.apply_assembly_activation_control(
            transition(
                "commit",
                reference.clone(),
                config_snapshot.clone(),
                replica,
            ),
            &resolver,
            &config_resolver,
        )
        .await
        .unwrap();
    }
    let first_registration = first.active_assembly_registration().unwrap().unwrap();
    let second_registration = second.active_assembly_registration().unwrap().unwrap();
    assert_ne!(first_registration, second_registration);

    let staged_successor = AssemblyActivationControl::Prepare {
        environment: "prod".to_string(),
        activation_id: "activation-2".to_string(),
        expected_generation: 1,
        candidate_generation: 2,
        assembly: reference.clone(),
        config_snapshot: config_snapshot.clone(),
        replica_id: "runtime-a".to_string(),
        service_db: None,
    };
    assert!(matches!(
        first
            .apply_assembly_activation_control(
                staged_successor.clone(),
                &resolver,
                &config_resolver,
            )
            .await
            .unwrap(),
        Some(AssemblyActivationControl::Prepared { .. })
    ));
    assert_eq!(
        first.active_assembly_registration().unwrap().unwrap(),
        first_registration,
        "prepare must not switch or register the staged generation"
    );
    let abort = match staged_successor {
        AssemblyActivationControl::Prepare {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
            ..
        } => AssemblyActivationControl::Abort {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
        },
        _ => unreachable!(),
    };
    first
        .apply_assembly_activation_control(abort.clone(), &resolver, &config_resolver)
        .await
        .unwrap();
    first
        .apply_assembly_activation_control(abort, &resolver, &config_resolver)
        .await
        .expect("abort replay must be idempotent");
    assert_eq!(
        first.active_assembly_registration().unwrap().unwrap(),
        first_registration
    );

    let unknown = RuntimeAssemblyRef {
        assembly_identity: AssemblyIdentity::new(format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            "b".repeat(64)
        )),
    };
    let rejected = first
        .apply_assembly_activation_control(
            AssemblyActivationControl::Prepare {
                environment: "prod".to_string(),
                activation_id: "activation-2".to_string(),
                expected_generation: 1,
                candidate_generation: 2,
                assembly: unknown,
                config_snapshot,
                replica_id: "runtime-a".to_string(),
                service_db: None,
            },
            &resolver,
            &config_resolver,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        rejected,
        AssemblyActivationControl::Reject {
            reason: AssemblyActivationRejectReason::Resolve,
            ..
        }
    ));
    assert_eq!(
        first.active_assembly_registration().unwrap().unwrap(),
        first_registration
    );

    drop(first);
    assert_eq!(
        second.active_assembly_registration().unwrap().unwrap(),
        second_registration
    );
}
