use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use skiff_artifact_model::{
    CanonicalPackageLinkPlan, FileIrRef, FileIrUnit, PackageArtifact, PackageArtifactRef,
    PublicationResourceRef, ServiceContract, ServiceDeployment, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};

use super::*;

mod execution;
mod full_chain;
mod recovery;

#[derive(Default)]
struct NoContentResolver {
    reads: AtomicUsize,
}

impl NoContentResolver {
    fn unexpected<T>(&self, kind: &str) -> anyhow::Result<T> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("assembly attempted an unexpected {kind} content read")
    }
}

impl RuntimeAssemblyContentResolver for NoContentResolver {
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
        global_ingress: Vec::new(),
    };
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut assembly).unwrap();
    assembly
}

fn invalid_assembly(identity: &str) -> RuntimeAssembly {
    let mut assembly = empty_assembly();
    assembly.assembly_identity = AssemblyIdentity::new(identity);
    assembly
}

#[tokio::test]
async fn assembly_admission_canonical_empty_becomes_active_without_content_reads() {
    let controller = AssemblyAdmissionController::default();
    let resolver = NoContentResolver::default();
    let expected_identity = empty_assembly().assembly_identity;

    let admitted = controller
        .admit(empty_assembly(), &resolver)
        .await
        .expect("canonical empty assembly should admit");

    assert_eq!(admitted.identity(), &expected_identity);
    assert_eq!(admitted.generation(), 1);
    assert!(admitted.is_empty());
    assert!(admitted.contract_store().is_empty());
    assert!(Arc::ptr_eq(
        admitted.contract_store(),
        admitted.candidate().contract_store()
    ));
    assert_eq!(resolver.reads.load(Ordering::SeqCst), 0);
    assert!(controller
        .route(&IngressSelector {
            protocol: skiff_artifact_model::IngressProtocol::Http,
            host: "missing.test".to_string(),
            method: Some("GET".to_string()),
            path: "/missing".to_string(),
        })
        .unwrap()
        .is_none());

    let health = controller.health().unwrap();
    assert_eq!(health.active_identity.as_ref(), Some(&expected_identity));
    assert_eq!(health.active_generation, Some(1));
    assert!(health.active_admitted_at.is_some());
    assert!(health.candidate.is_none());
    let outcome = health.last_outcome.unwrap();
    assert!(outcome.succeeded);
    assert_eq!(outcome.stage, AssemblyCandidateStage::Admit);
    assert!(outcome.error.is_none());
}

#[tokio::test]
async fn atomic_reload_load_failure_preserves_active_and_redacts_health_error() {
    let controller = AssemblyAdmissionController::default();
    let active = controller
        .admit(empty_assembly(), &NoContentResolver::default())
        .await
        .unwrap();
    let candidate = invalid_assembly("candidate-b-invalid-identity");
    let candidate_identity = candidate.assembly_identity.clone();
    let resolver = NoContentResolver::default();

    let error = controller.admit(candidate, &resolver).await.unwrap_err();

    assert!(error
        .to_string()
        .contains("whole-assembly candidate load failed"));
    assert_eq!(resolver.reads.load(Ordering::SeqCst), 0);
    let still_active = controller.active().unwrap().unwrap();
    assert!(Arc::ptr_eq(&active, &still_active));
    let health = controller.health().unwrap();
    assert_eq!(health.active_generation, Some(1));
    assert!(health.candidate.is_none());
    let outcome = health.last_outcome.unwrap();
    assert_eq!(outcome.generation, 2);
    assert_eq!(outcome.identity, candidate_identity);
    assert_eq!(outcome.stage, AssemblyCandidateStage::Load);
    assert!(!outcome.succeeded);
    assert_eq!(outcome.error.as_deref(), Some("whole-assembly load failed"));
    assert!(!outcome
        .error
        .unwrap()
        .contains("candidate-b-invalid-identity"));
}

#[tokio::test]
async fn atomic_reload_keeps_inflight_reader_pinned_to_one_active_generation() {
    let controller = AssemblyAdmissionController::default();
    let pinned = controller
        .admit(empty_assembly(), &NoContentResolver::default())
        .await
        .unwrap();

    let replacement = controller
        .admit(empty_assembly(), &NoContentResolver::default())
        .await
        .unwrap();

    assert_eq!(pinned.generation(), 1);
    assert_eq!(replacement.generation(), 2);
    assert_eq!(pinned.identity(), replacement.identity());
    assert!(!Arc::ptr_eq(&pinned, &replacement));
    assert!(Arc::ptr_eq(
        &replacement,
        &controller.active().unwrap().unwrap()
    ));
    assert!(pinned.is_empty());
}

#[tokio::test]
async fn atomic_reload_failure_at_every_candidate_stage_keeps_active() {
    let controller = AssemblyAdmissionController::default();
    let active = controller
        .admit(empty_assembly(), &NoContentResolver::default())
        .await
        .unwrap();

    for stage in [
        AssemblyCandidateStage::Load,
        AssemblyCandidateStage::Link,
        AssemblyCandidateStage::Validate,
        AssemblyCandidateStage::Admit,
    ] {
        let identity = AssemblyIdentity::new(format!("candidate-{}", stage.as_str()));
        let generation = controller.begin_candidate(identity.clone()).unwrap();
        for next in [
            AssemblyCandidateStage::Link,
            AssemblyCandidateStage::Validate,
            AssemblyCandidateStage::Admit,
        ]
        .into_iter()
        .take(stage.ordinal())
        {
            controller.advance_candidate(generation, next).unwrap();
        }
        controller
            .fail_candidate(generation, &identity, stage)
            .unwrap();

        assert!(Arc::ptr_eq(&active, &controller.active().unwrap().unwrap()));
        let health = controller.health().unwrap();
        assert_eq!(health.active_generation, Some(1));
        assert!(health.candidate.is_none());
        assert_eq!(health.last_outcome.unwrap().stage, stage);
    }
}

#[tokio::test]
async fn atomic_reload_serializes_concurrent_candidate_generations() {
    let controller = AssemblyAdmissionController::default();
    let active = controller
        .admit(empty_assembly(), &NoContentResolver::default())
        .await
        .unwrap();

    let reload = controller.reload.lock().await;
    let identity_b = AssemblyIdentity::new("candidate-b");
    let generation_b = controller.begin_candidate(identity_b.clone()).unwrap();
    controller
        .advance_candidate(generation_b, AssemblyCandidateStage::Link)
        .unwrap();

    let candidate_c = invalid_assembly("candidate-c-invalid-identity");
    let identity_c = candidate_c.assembly_identity.clone();
    let resolver_c = NoContentResolver::default();
    let mut pending_c = Box::pin(controller.admit(candidate_c, &resolver_c));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), pending_c.as_mut())
            .await
            .is_err(),
        "candidate C must wait for the serialized reload permit"
    );

    let while_c_waits = controller.health().unwrap();
    assert_eq!(while_c_waits.active_generation, Some(1));
    let visible_candidate = while_c_waits.candidate.unwrap();
    assert_eq!(visible_candidate.generation, 2);
    assert_eq!(visible_candidate.identity, identity_b);
    assert_eq!(visible_candidate.stage, AssemblyCandidateStage::Link);
    assert_eq!(resolver_c.reads.load(Ordering::SeqCst), 0);

    controller
        .fail_candidate(generation_b, &identity_b, AssemblyCandidateStage::Link)
        .unwrap();
    drop(reload);
    assert!(pending_c.await.is_err());

    assert!(Arc::ptr_eq(&active, &controller.active().unwrap().unwrap()));
    let final_health = controller.health().unwrap();
    assert_eq!(final_health.active_generation, Some(1));
    assert!(final_health.candidate.is_none());
    let outcome = final_health.last_outcome.unwrap();
    assert_eq!(outcome.generation, 3);
    assert_eq!(outcome.identity, identity_c);
}
