use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use skiff_artifact_model::{
    AssemblyActivationControl, AssemblyActivationRejectReason, RuntimeAssemblyRef,
    RuntimeConfigSnapshotRef,
};
use skiff_runtime_capability_context::{
    CancellationSource, DbCapabilityError, DbCapabilityFuture, DbCapabilityResult,
    DbCapabilitySource, DbProviderBuildInput, DbProviderFactory, DbProviderSource,
};
use tokio::sync::Notify;

use super::*;

#[derive(Clone, Default)]
struct AdmissionGateDbProvider {
    provisioned: Arc<Mutex<Vec<Vec<DbProviderBuildInput>>>>,
    built: Arc<Mutex<Vec<DbProviderBuildInput>>>,
    failure: Arc<Mutex<Option<String>>>,
}

impl AdmissionGateDbProvider {
    fn fail_with(&self, message: &str) {
        *self.failure.lock().unwrap() = Some(message.to_string());
    }
}

impl DbProviderFactory for AdmissionGateDbProvider {
    fn build(&self, input: DbProviderBuildInput) -> DbCapabilityResult<DbCapabilitySource> {
        self.built.lock().unwrap().push(input);
        Ok(DbCapabilitySource::unavailable())
    }

    fn provision<'a>(&'a self, inputs: Vec<DbProviderBuildInput>) -> DbCapabilityFuture<'a, ()> {
        let provisioned = Arc::clone(&self.provisioned);
        let failure = Arc::clone(&self.failure);
        Box::pin(async move {
            provisioned.lock().unwrap().push(inputs);
            match failure.lock().unwrap().clone() {
                Some(message) => Err(DbCapabilityError::decode(message)),
                None => Ok(()),
            }
        })
    }
}

#[derive(Clone, Default)]
struct BlockingProvisionProvider {
    blocking: Arc<AtomicBool>,
    started: Arc<Notify>,
    dropped: Arc<Notify>,
}

impl DbProviderFactory for BlockingProvisionProvider {
    fn build(&self, _input: DbProviderBuildInput) -> DbCapabilityResult<DbCapabilitySource> {
        Ok(DbCapabilitySource::unavailable())
    }

    fn provision<'a>(&'a self, _inputs: Vec<DbProviderBuildInput>) -> DbCapabilityFuture<'a, ()> {
        let blocking = Arc::clone(&self.blocking);
        let started = Arc::clone(&self.started);
        let dropped = Arc::clone(&self.dropped);
        Box::pin(async move {
            if !blocking.load(Ordering::Acquire) {
                return Ok(());
            }
            struct DropNotification(Arc<Notify>);
            impl Drop for DropNotification {
                fn drop(&mut self) {
                    self.0.notify_one();
                }
            }
            let _drop_notification = DropNotification(dropped);
            started.notify_one();
            std::future::pending::<()>().await;
            Ok(())
        })
    }
}

fn prepare_control(
    activation_id: &str,
    expected_generation: u64,
    candidate_generation: u64,
    assembly: RuntimeAssemblyRef,
    config_snapshot: RuntimeConfigSnapshotRef,
    replica_id: &str,
) -> AssemblyActivationControl {
    AssemblyActivationControl::Prepare {
        profile: "fixture".to_string(),
        activation_id: activation_id.to_string(),
        expected_generation,
        candidate_generation,
        assembly,
        config_snapshot,
        replica_id: replica_id.to_string(),
        service_db: None,
    }
}

#[tokio::test]
async fn whole_candidate_db_provisioning_completes_before_prepared_ack() {
    let fixture = CollectionIdentityFixture::new(BTreeMap::new(), None);
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let (config_snapshot, config_resolver) =
        config_snapshot_for_assembly("fixture", &fixture.assembly, &fixture.resolver);
    let provider = AdmissionGateDbProvider::default();
    let controller = AssemblyAdmissionController::new(
        "runtime-index-prepare",
        DbProviderSource::new(provider.clone()),
    );

    let reply = controller
        .apply_activation_control(
            prepare_control(
                "index-prepare",
                0,
                1,
                reference,
                config_snapshot,
                "runtime-index-prepare",
            ),
            &fixture.resolver,
            &config_resolver,
            Some(&mapping_service_db()),
        )
        .await
        .expect("prepare control should return a reply")
        .expect("prepare control should return a terminal reply");

    assert!(matches!(
        reply,
        AssemblyActivationControl::Prepared {
            candidate_generation: 1,
            ..
        }
    ));
    let provisioned = provider.provisioned.lock().unwrap();
    assert_eq!(provisioned.len(), 1);
    assert_eq!(provisioned[0].len(), 1);
    assert_eq!(provisioned[0][0].environment, "fixture");
    assert_eq!(
        provisioned[0][0].service_id,
        fixture.assembly.roots[0].service_id
    );
    assert_eq!(
        provider.built.lock().unwrap().as_slice(),
        provisioned[0].as_slice()
    );
    assert!(
        controller.active().unwrap().is_none(),
        "prepare must stage only; commit remains the publication point"
    );
}

#[tokio::test]
async fn cancelled_prepare_drops_pending_provision_and_preserves_committed_generation() {
    let fixture = CollectionIdentityFixture::new(BTreeMap::new(), None);
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let (config_snapshot, config_resolver) =
        config_snapshot_for_assembly("fixture", &fixture.assembly, &fixture.resolver);
    let provider = BlockingProvisionProvider::default();
    let controller = AssemblyAdmissionController::new(
        "runtime-index-cancel",
        DbProviderSource::new(provider.clone()),
    );
    let active = controller
        .recover_committed(
            "fixture",
            1,
            &reference,
            &config_snapshot,
            &fixture.resolver,
            &config_resolver,
            Some(&mapping_service_db()),
        )
        .await
        .expect("baseline committed generation should recover");
    provider.blocking.store(true, Ordering::Release);

    let cancellation = CancellationSource::new();
    let cancellation_token = cancellation.token();
    let service_db = mapping_service_db();
    let prepare_control = prepare_control(
        "index-cancel",
        1,
        2,
        reference.clone(),
        config_snapshot.clone(),
        "runtime-index-cancel",
    );
    let prepare = controller.apply_cancellable_activation_control(
        prepare_control.clone(),
        &fixture.resolver,
        &config_resolver,
        Some(&service_db),
        &cancellation_token,
    );
    tokio::pin!(prepare);
    tokio::select! {
        () = provider.started.notified() => {}
        result = &mut prepare => panic!("blocking provision completed before cancellation: {result:?}"),
    }

    cancellation.cancel();
    let reply = tokio::time::timeout(Duration::from_millis(100), &mut prepare)
        .await
        .expect("cancelled prepare must finish within 100ms")
        .expect("cancelled prepare must cleanly complete");
    assert!(
        reply.is_none(),
        "cancelled prepare must not emit Prepared or Reject"
    );
    tokio::time::timeout(Duration::from_millis(100), provider.dropped.notified())
        .await
        .expect("cancellation must drop the pending provider future");

    let health = controller.health().expect("admission health");
    assert!(health.candidate.is_none());
    assert!(health.last_outcome.is_none());
    let current = controller.active().unwrap().unwrap();
    assert!(Arc::ptr_eq(&active, &current));
    assert_eq!(current.generation(), 1);

    let abort = match prepare_control {
        AssemblyActivationControl::Prepare {
            profile,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
            ..
        } => AssemblyActivationControl::Abort {
            profile,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
        },
        _ => unreachable!(),
    };
    assert!(controller
        .apply_activation_control(
            abort,
            &fixture.resolver,
            &config_resolver,
            Some(&mapping_service_db()),
        )
        .await
        .expect("durable abort after prepare cancellation must be idempotent")
        .is_none());
}

#[tokio::test]
async fn index_reconciliation_failures_reject_prepare_and_keep_old_generation_active() {
    let fixture = CollectionIdentityFixture::new(BTreeMap::new(), None);
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let (config_snapshot, config_resolver) =
        config_snapshot_for_assembly("fixture", &fixture.assembly, &fixture.resolver);
    let provider = AdmissionGateDbProvider::default();
    let controller = AssemblyAdmissionController::new(
        "runtime-index-reject",
        DbProviderSource::new(provider.clone()),
    );
    controller
        .recover_committed(
            "fixture",
            1,
            &reference,
            &config_snapshot,
            &fixture.resolver,
            &config_resolver,
            Some(&mapping_service_db()),
        )
        .await
        .expect("initial committed generation should recover");
    let active = controller.active().unwrap().unwrap();
    assert_eq!(active.generation(), 1);

    for (case, failure) in [
        ("unique-duplicate", "unique index duplicate data"),
        ("managed-mismatch", "managed index definition mismatch"),
        ("stale-managed", "stale managed index"),
    ] {
        provider.fail_with(failure);
        let build_count = provider.built.lock().unwrap().len();
        let reply = controller
            .apply_activation_control(
                prepare_control(
                    case,
                    1,
                    2,
                    reference.clone(),
                    config_snapshot.clone(),
                    "runtime-index-reject",
                ),
                &fixture.resolver,
                &config_resolver,
                Some(&mapping_service_db()),
            )
            .await
            .expect("provisioning failure should produce a reject reply")
            .expect("prepare must return reject");
        assert!(matches!(
            reply,
            AssemblyActivationControl::Reject {
                reason: AssemblyActivationRejectReason::Admission,
                ..
            }
        ));
        assert_eq!(
            provider.built.lock().unwrap().len(),
            build_count,
            "capability sources must not build after provisioning fails"
        );
        assert!(Arc::ptr_eq(&active, &controller.active().unwrap().unwrap()));
        assert_eq!(controller.active().unwrap().unwrap().generation(), 1);
    }
}

#[tokio::test]
async fn cold_recovery_reconciles_indexes_before_publishing_and_no_db_skips_provider() {
    let stateful = CollectionIdentityFixture::new(BTreeMap::new(), None);
    let stateful_reference =
        skiff_artifact_identity::runtime_assembly_ref(&stateful.assembly).unwrap();
    let (stateful_snapshot, stateful_config_resolver) =
        config_snapshot_for_assembly("fixture", &stateful.assembly, &stateful.resolver);
    let provider = AdmissionGateDbProvider::default();
    let controller = AssemblyAdmissionController::new(
        "runtime-index-recovery",
        DbProviderSource::new(provider.clone()),
    );

    let recovered = controller
        .recover_committed(
            "fixture",
            7,
            &stateful_reference,
            &stateful_snapshot,
            &stateful.resolver,
            &stateful_config_resolver,
            Some(&mapping_service_db()),
        )
        .await
        .expect("cold recovery should provision before publication");
    assert_eq!(recovered.generation(), 7);
    assert_eq!(provider.provisioned.lock().unwrap().len(), 1);

    let stateless = FullChainFixture::new();
    let stateless_reference =
        skiff_artifact_identity::runtime_assembly_ref(&stateless.assembly).unwrap();
    let (stateless_snapshot, stateless_config_resolver) =
        config_snapshot_for_assembly("fixture", &stateless.assembly, &stateless.resolver);
    let stateless_provider = AdmissionGateDbProvider::default();
    stateless_provider.fail_with("provider must not be called for a stateless candidate");
    let stateless_controller = AssemblyAdmissionController::new(
        "runtime-stateless-recovery",
        DbProviderSource::new(stateless_provider.clone()),
    );
    stateless_controller
        .recover_committed(
            "fixture",
            0,
            &stateless_reference,
            &stateless_snapshot,
            &stateless.resolver,
            &stateless_config_resolver,
            None,
        )
        .await
        .expect("stateless cold recovery must not require serviceDb");
    assert!(stateless_provider.provisioned.lock().unwrap().is_empty());
    assert!(stateless_provider.built.lock().unwrap().is_empty());
}

#[tokio::test]
async fn every_runtime_replica_runs_the_idempotent_whole_candidate_gate() {
    let fixture = CollectionIdentityFixture::new(BTreeMap::new(), None);
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let (config_snapshot, config_resolver) =
        config_snapshot_for_assembly("fixture", &fixture.assembly, &fixture.resolver);
    let provider = AdmissionGateDbProvider::default();
    let first = AssemblyAdmissionController::new(
        "runtime-index-replica-a",
        DbProviderSource::new(provider.clone()),
    );
    let second = AssemblyAdmissionController::new(
        "runtime-index-replica-b",
        DbProviderSource::new(provider.clone()),
    );
    let service_db = mapping_service_db();

    let (first_result, second_result) = tokio::join!(
        first.recover_committed(
            "fixture",
            9,
            &reference,
            &config_snapshot,
            &fixture.resolver,
            &config_resolver,
            Some(&service_db),
        ),
        second.recover_committed(
            "fixture",
            9,
            &reference,
            &config_snapshot,
            &fixture.resolver,
            &config_resolver,
            Some(&service_db),
        )
    );
    assert_eq!(first_result.unwrap().generation(), 9);
    assert_eq!(second_result.unwrap().generation(), 9);
    assert_eq!(
        provider.provisioned.lock().unwrap().len(),
        2,
        "every replica must independently await the provider's idempotent gate"
    );
}

#[tokio::test]
async fn provisioning_root_cause_stays_visible_in_recovery_error_chain() {
    let fixture = CollectionIdentityFixture::new(BTreeMap::new(), None);
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let (config_snapshot, config_resolver) =
        config_snapshot_for_assembly("fixture", &fixture.assembly, &fixture.resolver);
    let provider = AdmissionGateDbProvider::default();
    // Opaque provider failures can Display as an empty string; the whole-candidate
    // recovery error must never collapse to an empty root cause.
    provider.fail_with("");
    let controller = AssemblyAdmissionController::new(
        "runtime-chain-root-cause",
        DbProviderSource::new(provider.clone()),
    );

    let error = controller
        .recover_committed(
            "fixture",
            1,
            &reference,
            &config_snapshot,
            &fixture.resolver,
            &config_resolver,
            Some(&mapping_service_db()),
        )
        .await
        .expect_err("whole-candidate provisioning failure must reject cold recovery");

    let chain = format!("{error:#}");
    assert!(
        chain.contains("whole-assembly activation context construction failed"),
        "outer admission context must remain in the chain: {chain}"
    );
    assert!(
        chain.contains("whole-assembly service DB index provisioning failed"),
        "provisioning context must remain in the chain: {chain}"
    );
    assert!(
        chain.contains("Decode"),
        "empty-Display provider failure must stay visible through the Debug fallback: {chain}"
    );
}
