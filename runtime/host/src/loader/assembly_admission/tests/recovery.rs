use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use skiff_artifact_model::*;

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

fn generation_one_control(kind: &str, assembly: RuntimeAssemblyRef) -> AssemblyActivationControl {
    let common = (
        "prod".to_string(),
        "activation-1".to_string(),
        0,
        1,
        assembly,
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
                replica_id,
            ),
        ) => AssemblyActivationControl::Prepare {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            replica_id,
        },
        (
            "abort",
            (
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
            ),
        ) => AssemblyActivationControl::Abort {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
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
                replica_id,
            ),
        ) => AssemblyActivationControl::Commit {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            replica_id,
        },
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn committed_recovery_generation_zero_rebuilds_and_preserves_online_transaction_rules() {
    let resolver = EmptyRecordResolver::new(empty_assembly());
    let reference = skiff_artifact_identity::runtime_assembly_ref(&resolver.assembly).unwrap();
    let controller = AssemblyAdmissionController::new("runtime-a");

    let first = controller
        .recover_committed("prod", 0, &reference, &resolver)
        .await
        .expect("canonical generation zero must recover");
    let second = controller
        .recover_committed("prod", 0, &reference, &resolver)
        .await
        .expect("every reconnect must rebuild the exact durable record");
    assert_eq!(first.generation(), 0);
    assert_eq!(second.generation(), 0);
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(resolver.record_reads.load(Ordering::SeqCst), 2);
    assert!(controller
        .begin_online_candidate(0, reference.assembly_identity.clone())
        .is_err());

    let prepare = generation_one_control("prepare", reference.clone());
    assert!(matches!(
        controller
            .apply_activation_control(prepare.clone(), &resolver)
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
            generation_one_control("abort", reference.clone()),
            &resolver,
        )
        .await
        .unwrap();
    assert!(matches!(
        controller.registration().unwrap(),
        Some(AssemblyActivationControl::Register { generation: 0, .. })
    ));

    controller
        .apply_activation_control(prepare, &resolver)
        .await
        .unwrap();
    let commit = generation_one_control("commit", reference.clone());
    assert!(matches!(
        controller
            .apply_activation_control(commit.clone(), &resolver)
            .await
            .unwrap(),
        Some(AssemblyActivationControl::Register { generation: 1, .. })
    ));
    assert_eq!(controller.active().unwrap().unwrap().generation(), 1);
    assert_eq!(resolver.record_reads.load(Ordering::SeqCst), 4);

    let replayed = AssemblyAdmissionController::new("runtime-a");
    replayed
        .recover_committed("prod", 0, &reference, &resolver)
        .await
        .unwrap();
    assert!(matches!(
        replayed
            .apply_activation_control(commit, &resolver)
            .await
            .unwrap(),
        Some(AssemblyActivationControl::Register { generation: 1, .. })
    ));
    assert_eq!(replayed.active().unwrap().unwrap().generation(), 1);
    assert_eq!(resolver.record_reads.load(Ordering::SeqCst), 6);
    assert_eq!(resolver.content_reads.load(Ordering::SeqCst), 0);
}
