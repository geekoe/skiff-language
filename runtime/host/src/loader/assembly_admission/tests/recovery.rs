use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use skiff_artifact_model::*;

use crate::loader::config_snapshot::snapshot_for_assembly as config_snapshot_for_assembly;

use super::{super::*, empty_assembly};

struct EmptyRecordResolver {
    assembly: Arc<RuntimeAssembly>,
    record_reads: AtomicUsize,
    content_reads: AtomicUsize,
}

impl EmptyRecordResolver {
    fn new(assembly: RuntimeAssembly) -> Self {
        Self {
            assembly: Arc::new(assembly),
            record_reads: AtomicUsize::new(0),
            content_reads: AtomicUsize::new(0),
        }
    }

    fn unexpected<T>(&self, kind: &str) -> anyhow::Result<T> {
        self.content_reads.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("empty assembly attempted an unexpected {kind} content read")
    }
}

impl RuntimeAssemblyRecordResolver for EmptyRecordResolver {
    fn resolve_runtime_assembly(
        &self,
        _reference: &RuntimeAssemblyRef,
    ) -> anyhow::Result<Arc<RuntimeAssembly>> {
        self.record_reads.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::clone(&self.assembly))
    }
}

impl RuntimeAssemblyContentResolver for EmptyRecordResolver {
    fn resolve_deployment(
        &self,
        _reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        self.unexpected("deployment")
    }

    fn resolve_contract(
        &self,
        _reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        self.unexpected("contract")
    }

    fn resolve_package_schema_type(
        &self,
        _reference: &skiff_artifact_model::PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::PackageSchemaTypeRecord>> {
        self.unexpected("package schema")
    }

    fn resolve_package(
        &self,
        _reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        self.unexpected("package")
    }

    fn resolve_file_ir(
        &self,
        _package: &PackageArtifactRef,
        _reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        self.unexpected("File IR")
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        _reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        self.unexpected("resource")
    }
}

fn generation_one_control(
    kind: &str,
    assembly: RuntimeAssemblyRef,
    config_snapshot: RuntimeConfigSnapshotRef,
) -> AssemblyActivationControl {
    let common = (
        "prod".to_string(),
        "activation-1".to_string(),
        0,
        1,
        assembly,
        config_snapshot,
        "runtime-a".to_string(),
    );
    match (kind, common) {
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
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn committed_recovery_generation_zero_rebuilds_and_preserves_online_transaction_rules() {
    let resolver = EmptyRecordResolver::new(empty_assembly());
    let reference = skiff_artifact_identity::runtime_assembly_ref(&resolver.assembly).unwrap();
    let (config_snapshot, config_resolver) =
        config_snapshot_for_assembly(&resolver.assembly, &resolver);
    let controller = AssemblyAdmissionController::new(
        "runtime-a",
        skiff_runtime_capability_context::DbProviderSource::unavailable(),
    );

    let first = controller
        .recover_committed(
            "prod",
            0,
            &reference,
            &config_snapshot,
            &resolver,
            &config_resolver,
            None,
        )
        .await
        .expect("canonical generation zero must recover");
    let second = controller
        .recover_committed(
            "prod",
            0,
            &reference,
            &config_snapshot,
            &resolver,
            &config_resolver,
            None,
        )
        .await
        .expect("every reconnect must rebuild the exact durable record");
    assert_eq!(first.generation(), 0);
    assert_eq!(second.generation(), 0);
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(resolver.record_reads.load(Ordering::SeqCst), 2);
    assert!(controller
        .begin_online_candidate(0, reference.assembly_identity.clone())
        .is_err());

    let prepare = generation_one_control("prepare", reference.clone(), config_snapshot.clone());
    assert!(matches!(
        controller
            .apply_activation_control(prepare.clone(), &resolver, &config_resolver, None)
            .await
            .unwrap(),
        Some(AssemblyActivationControl::Prepared { .. })
    ));
    assert!(matches!(
        controller.registration().unwrap(),
        Some(AssemblyActivationControl::Register { generation: 0, .. })
    ));
    controller
        .apply_activation_control(
            generation_one_control("abort", reference.clone(), config_snapshot.clone()),
            &resolver,
            &config_resolver,
            None,
        )
        .await
        .unwrap();
    assert!(matches!(
        controller.registration().unwrap(),
        Some(AssemblyActivationControl::Register { generation: 0, .. })
    ));

    controller
        .apply_activation_control(prepare, &resolver, &config_resolver, None)
        .await
        .unwrap();
    let commit = generation_one_control("commit", reference.clone(), config_snapshot.clone());
    assert!(matches!(
        controller
            .apply_activation_control(commit.clone(), &resolver, &config_resolver, None)
            .await
            .unwrap(),
        Some(AssemblyActivationControl::Register { generation: 1, .. })
    ));
    assert_eq!(controller.active().unwrap().unwrap().generation(), 1);
    assert_eq!(resolver.record_reads.load(Ordering::SeqCst), 4);

    let replayed = AssemblyAdmissionController::new(
        "runtime-a",
        skiff_runtime_capability_context::DbProviderSource::unavailable(),
    );
    replayed
        .recover_committed(
            "prod",
            0,
            &reference,
            &config_snapshot,
            &resolver,
            &config_resolver,
            None,
        )
        .await
        .unwrap();
    assert!(matches!(
        replayed
            .apply_activation_control(commit, &resolver, &config_resolver, None,)
            .await
            .unwrap(),
        Some(AssemblyActivationControl::Register { generation: 1, .. })
    ));
    assert_eq!(replayed.active().unwrap().unwrap().generation(), 1);
    assert_eq!(resolver.record_reads.load(Ordering::SeqCst), 6);
    assert_eq!(resolver.content_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn activation_and_recovery_pin_assembly_and_config_snapshot_as_one_exact_pair() {
    let resolver = EmptyRecordResolver::new(empty_assembly());
    let assembly = skiff_artifact_identity::runtime_assembly_ref(&resolver.assembly).unwrap();
    let (snapshot_a, resolver_a) = config_snapshot_for_assembly(&resolver.assembly, &resolver);
    let (snapshot_b, resolver_b) = config_snapshot_for_assembly(&resolver.assembly, &resolver);
    let controller = AssemblyAdmissionController::new(
        "runtime-a",
        skiff_runtime_capability_context::DbProviderSource::unavailable(),
    );

    controller
        .apply_activation_control(
            generation_one_control("prepare", assembly.clone(), snapshot_a.clone()),
            &resolver,
            &resolver_a,
            None,
        )
        .await
        .unwrap();

    let mismatched_commit = controller
        .apply_activation_control(
            generation_one_control("commit", assembly.clone(), snapshot_b.clone()),
            &resolver,
            &resolver_b,
            None,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        mismatched_commit,
        AssemblyActivationControl::Reject {
            reason: AssemblyActivationRejectReason::Admission,
            ..
        }
    ));
    assert!(controller.active().unwrap().is_none());

    let mismatched_abort = controller
        .apply_activation_control(
            generation_one_control("abort", assembly.clone(), snapshot_b.clone()),
            &resolver,
            &resolver_b,
            None,
        )
        .await
        .unwrap_err();
    assert!(mismatched_abort
        .to_string()
        .contains("does not match the staged"));

    controller
        .apply_activation_control(
            generation_one_control("commit", assembly.clone(), snapshot_a.clone()),
            &resolver,
            &resolver_a,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        controller.active().unwrap().unwrap().config_snapshot(),
        &snapshot_a
    );

    let recovery_error = controller
        .recover_committed(
            "prod",
            1,
            &assembly,
            &snapshot_b,
            &resolver,
            &resolver_b,
            None,
        )
        .await
        .unwrap_err();
    assert!(recovery_error
        .to_string()
        .contains("config snapshot changed without generation advance"));
}

#[tokio::test]
async fn prepare_rejects_an_unresolvable_exact_config_snapshot_before_ack() {
    let resolver = EmptyRecordResolver::new(empty_assembly());
    let assembly = skiff_artifact_identity::runtime_assembly_ref(&resolver.assembly).unwrap();
    let (_available_snapshot, available_resolver) =
        config_snapshot_for_assembly(&resolver.assembly, &resolver);
    let (missing_snapshot, _missing_resolver) =
        config_snapshot_for_assembly(&resolver.assembly, &resolver);
    let controller = AssemblyAdmissionController::new(
        "runtime-a",
        skiff_runtime_capability_context::DbProviderSource::unavailable(),
    );

    let reply = controller
        .apply_activation_control(
            generation_one_control("prepare", assembly, missing_snapshot),
            &resolver,
            &available_resolver,
            None,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        reply,
        AssemblyActivationControl::Reject {
            reason: AssemblyActivationRejectReason::Resolve,
            ..
        }
    ));
    assert!(controller.active().unwrap().is_none());
}
