use std::sync::{atomic::Ordering, Arc};

use skiff_artifact_model::*;
use skiff_runtime_capability_context::DbProviderSource;

use super::*;

type TestConfigResolver = crate::loader::config_snapshot::TestSnapshotResolver;

/// Adds one additional self-contained deployment (no cross-service
/// dependencies) to the fixture resolver so it can be lazy-loaded under a
/// buildId that is not part of the recovered committed assembly.
fn extra_self_contained_deployment(fixture: &mut FullChainFixture, revision: &str) -> ServiceDeploymentRef {
    let provider = fixture
        .resolver
        .deployments
        .iter()
        .find(|(reference, _)| reference == &fixture.provider_deployment_ref)
        .expect("fixture provider deployment")
        .1
        .as_ref()
        .clone();
    let mut deployment = provider.clone();
    deployment.deployment_revision = DeploymentRevision::new(revision);
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment)
        .expect("extra deployment identity");
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    fixture
        .resolver
        .deployments
        .push((reference.clone(), Arc::new(deployment)));
    reference
}

fn missing_deployment_ref() -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: "example.missing".to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("missing-revision"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
            "skiff-deployment-artifact-v4:sha256:{}",
            "0".repeat(64)
        )),
    }
}

async fn recovered_controller(
    fixture: &FullChainFixture,
) -> (AssemblyAdmissionController, RuntimeConfigSnapshotRef, TestConfigResolver) {
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let (snapshot, snapshot_resolver) =
        config_snapshot_for_assembly("fixture", &fixture.assembly, &fixture.resolver);
    let controller = AssemblyAdmissionController::new(
        "runtime-lazy",
        DbProviderSource::unavailable(),
    );
    controller
        .recover_committed(
            "fixture",
            1,
            &reference,
            &snapshot,
            &fixture.resolver,
            &snapshot_resolver,
            None,
        )
        .await
        .expect("committed baseline should recover");
    (controller, snapshot, snapshot_resolver)
}

#[tokio::test]
async fn lazy_load_materializes_new_build_id_once_and_registers_it() {
    let mut fixture = FullChainFixture::new();
    let extra = extra_self_contained_deployment(&mut fixture, "lazy-revision-1");
    let (controller, _snapshot, snapshot_resolver) = recovered_controller(&fixture).await;
    assert!(!controller.is_loaded(extra.deployment_artifact_identity.as_str()));

    let reads_before = fixture.resolver.reads.load(Ordering::SeqCst);
    let loaded = controller
        .deployment_image_or_lazy_load(
            &extra,
            &fixture.resolver,
            &snapshot_resolver,
            None,
            "fixture",
        )
        .await
        .expect("first request must trigger the lazy load");
    assert!(fixture.resolver.reads.load(Ordering::SeqCst) > reads_before);
    assert!(controller.is_loaded(extra.deployment_artifact_identity.as_str()));
    assert_eq!(loaded.generation(), 0, "lazy loads carry no generation semantics");
    assert!(controller
        .loaded_build_ids()
        .contains(&extra.deployment_artifact_identity.as_str().to_string()));

    // Second request resolves from the loaded registry without any artifact IO.
    let reads_before = fixture.resolver.reads.load(Ordering::SeqCst);
    let again = controller
        .deployment_image_or_lazy_load(
            &extra,
            &fixture.resolver,
            &snapshot_resolver,
            None,
            "fixture",
        )
        .await
        .expect("second request must resolve the loaded image");
    assert_eq!(
        fixture.resolver.reads.load(Ordering::SeqCst),
        reads_before,
        "already-loaded buildId must not re-read artifact records"
    );
    assert!(Arc::ptr_eq(&loaded, &again));
}

#[tokio::test]
async fn lazy_load_fast_fails_on_missing_record_and_does_not_register() {
    let fixture = FullChainFixture::new();
    let (controller, _snapshot, snapshot_resolver) = recovered_controller(&fixture).await;
    let missing = missing_deployment_ref();

    let error = controller
        .deployment_image_or_lazy_load(
            &missing,
            &fixture.resolver,
            &snapshot_resolver,
            None,
            "fixture",
        )
        .await
        .expect_err("missing deployment record must fast-fail");
    assert!(format!("{error:#}").contains("failed to resolve deployment"));
    assert!(!controller.is_loaded(missing.deployment_artifact_identity.as_str()));

    // A failed load is not cached: the next request re-enters the critical
    // section and fails again.
    let error = controller
        .deployment_image_or_lazy_load(
            &missing,
            &fixture.resolver,
            &snapshot_resolver,
            None,
            "fixture",
        )
        .await
        .expect_err("missing deployment record must keep fast-failing");
    assert!(format!("{error:#}").contains("failed to resolve deployment"));
}

#[tokio::test]
async fn lazy_load_fast_fails_on_cross_service_dependencies() {
    let mut fixture = FullChainFixture::new();
    // The committed consumer is already loaded by recovery; build an unloaded
    // variant that still declares cross-service dependencies.
    let consumer = fixture
        .resolver
        .deployments
        .iter()
        .find(|(reference, _)| reference == &fixture.consumer_deployment_ref)
        .expect("fixture consumer deployment")
        .1
        .as_ref()
        .clone();
    let mut variant = consumer.clone();
    variant.deployment_revision = DeploymentRevision::new("consumer-lazy-revision");
    skiff_artifact_identity::assign_service_deployment_identity(&mut variant)
        .expect("variant deployment identity");
    let variant_ref = skiff_artifact_identity::service_deployment_ref(&variant);
    assert!(!variant.service_selectors.is_empty());
    fixture
        .resolver
        .deployments
        .push((variant_ref.clone(), Arc::new(variant)));
    let (controller, _snapshot, snapshot_resolver) = recovered_controller(&fixture).await;

    let error = controller
        .deployment_image_or_lazy_load(
            &variant_ref,
            &fixture.resolver,
            &snapshot_resolver,
            None,
            "fixture",
        )
        .await
        .expect_err("cross-service deployment must fast-fail");
    assert!(
        format!("{error:#}").contains("cross-service dependencies"),
        "unexpected error: {error:#}"
    );
    assert!(!controller.is_loaded(variant_ref.deployment_artifact_identity.as_str()));
}

#[tokio::test]
async fn concurrent_lazy_load_materializes_exactly_once() {
    let mut fixture = FullChainFixture::new();
    let extra = extra_self_contained_deployment(&mut fixture, "lazy-revision-concurrent");
    let extra_concurrent =
        extra_self_contained_deployment(&mut fixture, "lazy-revision-concurrent-2");
    let (controller, _snapshot, snapshot_resolver) = recovered_controller(&fixture).await;
    let controller = Arc::new(controller);
    let resolver = Arc::new(fixture.resolver);
    let snapshot_resolver = Arc::new(snapshot_resolver);
    let extra = Arc::new(extra);
    let extra_concurrent = Arc::new(extra_concurrent);

    // Serial first-load baseline for one full materialization.
    let reads_before = resolver.reads.load(Ordering::SeqCst);
    controller
        .deployment_image_or_lazy_load(
            &extra,
            resolver.as_ref(),
            snapshot_resolver.as_ref(),
            None,
            "fixture",
        )
        .await
        .expect("baseline lazy load");
    let single_load_reads = resolver.reads.load(Ordering::SeqCst) - reads_before;
    assert!(single_load_reads > 0);

    // Concurrent first load of a second fresh buildId: exactly one
    // materialization, every waiter observes the same image.
    let reads_before = resolver.reads.load(Ordering::SeqCst);
    let mut handles = Vec::new();
    for _ in 0..8 {
        let controller = Arc::clone(&controller);
        let resolver = Arc::clone(&resolver);
        let snapshot_resolver = Arc::clone(&snapshot_resolver);
        let extra = Arc::clone(&extra_concurrent);
        handles.push(tokio::spawn(async move {
            controller
                .deployment_image_or_lazy_load(
                    &extra,
                    resolver.as_ref(),
                    snapshot_resolver.as_ref(),
                    None,
                    "fixture",
                )
                .await
                .expect("concurrent lazy load must succeed")
        }));
    }
    let mut images = Vec::new();
    for handle in handles {
        images.push(handle.await.expect("concurrent lazy load task"));
    }
    let reads_after = resolver.reads.load(Ordering::SeqCst);
    assert_eq!(
        reads_after - reads_before,
        single_load_reads,
        "concurrent first requests must materialize exactly once"
    );
    for image in &images {
        assert!(Arc::ptr_eq(&images[0], image));
    }
    assert!(controller.is_loaded(
        extra_concurrent
            .deployment_artifact_identity
            .as_str()
    ));
}
