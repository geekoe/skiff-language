use std::sync::{atomic::Ordering, Arc};

use skiff_artifact_model::*;
use skiff_runtime_capability_context::DbProviderSource;

use super::*;

/// Adds one additional self-contained deployment (no cross-service
/// dependencies) to the fixture resolver so it can be lazy-loaded under a
/// buildId that is not part of the fixture assembly.
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

/// Adds an unloaded consumer variant that still declares the provider
/// service selector, so the lazy-load path must resolve the dependency
/// closure through the release pointer table.
fn unloaded_consumer_variant(
    fixture: &mut FullChainFixture,
    revision: &str,
) -> ServiceDeploymentRef {
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
    variant.deployment_revision = DeploymentRevision::new(revision);
    skiff_artifact_identity::assign_service_deployment_identity(&mut variant)
        .expect("variant deployment identity");
    let variant_ref = skiff_artifact_identity::service_deployment_ref(&variant);
    assert!(!variant.service_selectors.is_empty());
    fixture
        .resolver
        .deployments
        .push((variant_ref.clone(), Arc::new(variant)));
    variant_ref
}

/// Adds a provider variant that declares a selector back to the consumer
/// contract, producing a two-deployment dependency cycle when the release
/// pointers point at each other.
fn provider_variant_with_selector(
    fixture: &mut FullChainFixture,
    revision: &str,
    contract: ServiceContractRef,
) -> ServiceDeploymentRef {
    let provider = fixture
        .resolver
        .deployments
        .iter()
        .find(|(reference, _)| reference == &fixture.provider_deployment_ref)
        .expect("fixture provider deployment")
        .1
        .as_ref()
        .clone();
    let mut variant = provider.clone();
    variant.deployment_revision = DeploymentRevision::new(revision);
    variant.service_selectors = vec![ServiceSelectorBinding {
        key: ServiceRequirementKey {
            caller_package_build_id: variant.implementation.package_build_id.clone(),
            service_requirement_slot: 0,
        },
        contract,
    }];
    skiff_artifact_identity::assign_service_deployment_identity(&mut variant)
        .expect("variant deployment identity");
    let reference = skiff_artifact_identity::service_deployment_ref(&variant);
    fixture
        .resolver
        .deployments
        .push((reference.clone(), Arc::new(variant)));
    reference
}

fn consumer_contract_ref(fixture: &FullChainFixture) -> ServiceContractRef {
    fixture
        .resolver
        .deployments
        .iter()
        .find(|(reference, _)| reference == &fixture.consumer_deployment_ref)
        .expect("fixture consumer deployment")
        .1
        .contract
        .clone()
}

async fn fresh_controller() -> AssemblyAdmissionController {
    AssemblyAdmissionController::new("runtime-lazy", DbProviderSource::unavailable())
}

#[tokio::test]
async fn lazy_load_materializes_new_build_id_once_and_registers_it() {
    let mut fixture = FullChainFixture::new();
    let extra = extra_self_contained_deployment(&mut fixture, "lazy-revision-1");
    let controller = fresh_controller().await;
    assert!(!controller.is_loaded(extra.deployment_artifact_identity.as_str()));

    let reads_before = fixture.resolver.reads.load(Ordering::SeqCst);
    let loaded = controller
        .deployment_image_or_lazy_load(
            &extra,
            &fixture.resolver,
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
    let controller = fresh_controller().await;
    let missing = missing_deployment_ref();

    let error = controller
        .deployment_image_or_lazy_load(
            &missing,
            &fixture.resolver,
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
            None,
            "fixture",
        )
        .await
        .expect_err("missing deployment record must keep fast-failing");
    assert!(format!("{error:#}").contains("failed to resolve deployment"));
}

#[tokio::test]
async fn lazy_load_materializes_dependency_closure_for_cross_service_dependencies() {
    let mut fixture = FullChainFixture::new();
    let entry = unloaded_consumer_variant(&mut fixture, "consumer-lazy-revision");
    let controller = fresh_controller().await;
    assert!(!controller.is_loaded(entry.deployment_artifact_identity.as_str()));

    let reads_before = fixture.resolver.reads.load(Ordering::SeqCst);
    let loaded = controller
        .deployment_image_or_lazy_load(
            &entry,
            &fixture.resolver,
            None,
            "fixture",
        )
        .await
        .expect("cross-service entry must lazy-load its whole dependency closure");
    assert!(fixture.resolver.reads.load(Ordering::SeqCst) > reads_before);
    assert!(controller.is_loaded(entry.deployment_artifact_identity.as_str()));

    // The whole closure (entry + provider) is registered under one image, so
    // the capability advertisement covers the provider buildId too.
    let provider_build_id = fixture.provider_deployment_ref.deployment_artifact_identity.as_str();
    let loaded_build_ids = controller.loaded_build_ids();
    assert!(loaded_build_ids.contains(&entry.deployment_artifact_identity.as_str().to_string()));
    assert!(loaded_build_ids.contains(&provider_build_id.to_string()));

    // The closure image carries both activations, and the provider buildId
    // resolves to the very same loaded image without any artifact IO.
    assert_eq!(loaded.candidate().activations().len(), 2);
    assert!(loaded
        .candidate()
        .activation(&fixture.provider_deployment_ref)
        .is_some());
    let reads_before = fixture.resolver.reads.load(Ordering::SeqCst);
    let provider_image = controller
        .deployment_image_or_lazy_load(
            &fixture.provider_deployment_ref,
            &fixture.resolver,
            None,
            "fixture",
        )
        .await
        .expect("provider buildId must resolve from the loaded closure image");
    assert_eq!(
        fixture.resolver.reads.load(Ordering::SeqCst),
        reads_before,
        "closure provider buildId must not re-read artifact records"
    );
    // The lazy-load closure is the only registration source: the provider
    // buildId was registered by the closure image itself, so the resolved
    // image must be the very same closure image serving the provider
    // activation.
    assert!(
        Arc::ptr_eq(&provider_image, &loaded),
        "single closure image must own every buildId it registered"
    );
    assert!(
        provider_image
            .candidate()
            .activation(&fixture.provider_deployment_ref)
            .is_some(),
        "provider buildId must resolve to an image serving the provider activation"
    );

    // The linker binding inside the closure image pins the provider activation.
    let linked_call = loaded
        .candidate()
        .shared_image()
        .resolve_activation_relative_service_call(
            &fixture.consumer_package_ref.package_build_id,
            &fixture.consumer_file_ir_identity,
            ServiceCallRefIndex::new(0),
        )
        .expect("linked service call");
    let binding = loaded
        .candidate()
        .resolve_activation_relative_service_call(&entry, &linked_call)
        .expect("closure service binding");
    assert_eq!(binding.provider(), &fixture.provider_deployment_ref);
}

#[tokio::test]
async fn lazy_load_fast_fails_when_provider_release_pointer_is_missing() {
    let mut fixture = FullChainFixture::new();
    let entry = unloaded_consumer_variant(&mut fixture, "consumer-no-pointer");
    // Remove the provider release pointer: the closure cannot resolve.
    fixture.resolver.release_pointers.clear();
    let controller = fresh_controller().await;

    let error = controller
        .deployment_image_or_lazy_load(
            &entry,
            &fixture.resolver,
            None,
            "fixture",
        )
        .await
        .expect_err("missing provider release pointer must fast-fail");
    assert!(
        format!("{error:#}").contains("no release pointer"),
        "unexpected error: {error:#}"
    );
    assert!(!controller.is_loaded(entry.deployment_artifact_identity.as_str()));

    let error = controller
        .deployment_image_or_lazy_load(
            &entry,
            &fixture.resolver,
            None,
            "fixture",
        )
        .await
        .expect_err("missing provider release pointer must keep fast-failing");
    assert!(
        format!("{error:#}").contains("no release pointer"),
        "unexpected error: {error:#}"
    );
}

#[tokio::test]
async fn lazy_load_fast_fails_on_dependency_cycles_and_registers_nothing() {
    let mut fixture = FullChainFixture::new();
    // A selector chain that points back at the entry deployment.
    let entry = unloaded_consumer_variant(&mut fixture, "consumer-cycle");
    let consumer_contract = consumer_contract_ref(&fixture);
    let back = provider_variant_with_selector(&mut fixture, "provider-cycle", consumer_contract);
    let provider_contract = fixture
        .resolver
        .deployments
        .iter()
        .find(|(reference, _)| reference == &fixture.provider_deployment_ref)
        .expect("fixture provider deployment")
        .1
        .contract
        .clone();
    // Complete the cycle: consumer -> provider -> consumer. Both release
    // pointers must resolve so the closure walk hits the already-visited
    // entry and fails with a cycle.
    fixture
        .resolver
        .release_pointers
        .insert(
            (provider_contract.service_id, provider_contract.contract_version),
            back.clone(),
        );
    let consumer_contract = consumer_contract_ref(&fixture);
    fixture
        .resolver
        .release_pointers
        .insert(
            (consumer_contract.service_id, consumer_contract.contract_version),
            entry.clone(),
        );
    let controller = fresh_controller().await;

    let error = controller
        .deployment_image_or_lazy_load(
            &entry,
            &fixture.resolver,
            None,
            "fixture",
        )
        .await
        .expect_err("dependency cycle must fail closed");
    assert!(
        format!("{error:#}").contains("dependency cycle"),
        "unexpected error: {error:#}"
    );
    assert!(!controller.is_loaded(entry.deployment_artifact_identity.as_str()));
    assert!(!controller.is_loaded(back.deployment_artifact_identity.as_str()));
}

#[tokio::test]
async fn concurrent_lazy_load_materializes_exactly_once() {
    let mut fixture = FullChainFixture::new();
    let extra = extra_self_contained_deployment(&mut fixture, "lazy-revision-concurrent");
    let extra_concurrent =
        extra_self_contained_deployment(&mut fixture, "lazy-revision-concurrent-2");
    let controller = Arc::new(fresh_controller().await);
    let resolver = Arc::new(fixture.resolver);
    let extra = Arc::new(extra);
    let extra_concurrent = Arc::new(extra_concurrent);

    // Serial first-load baseline for one full materialization.
    let reads_before = resolver.reads.load(Ordering::SeqCst);
    controller
        .deployment_image_or_lazy_load(
            &extra,
            resolver.as_ref(),
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
        let extra = Arc::clone(&extra_concurrent);
        handles.push(tokio::spawn(async move {
            controller
                .deployment_image_or_lazy_load(
                    &extra,
                    resolver.as_ref(),
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
