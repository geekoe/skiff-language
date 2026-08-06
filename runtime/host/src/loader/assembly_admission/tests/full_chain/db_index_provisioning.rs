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
async fn prepare_ack_does_not_trigger_db_provisioning() {
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
    // M2: Prepare is wire compatibility only and must not load, materialize or
    // provision anything.
    assert!(
        provider.provisioned.lock().unwrap().is_empty(),
        "Prepare must not trigger DB provisioning"
    );
    assert!(
        provider.built.lock().unwrap().is_empty(),
        "Prepare must not build capability sources"
    );
    assert!(
        controller.active().unwrap().is_none(),
        "prepare must not publish any active assembly"
    );
}

#[tokio::test]
async fn prepare_ack_preserves_committed_generation_and_abort_is_idempotent() {
    let fixture = CollectionIdentityFixture::new(BTreeMap::new(), None);
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();
    let (config_snapshot, config_resolver) =
        config_snapshot_for_assembly("fixture", &fixture.assembly, &fixture.resolver);
    let provider = AdmissionGateDbProvider::default();
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
    let provisioned_before = provider.provisioned.lock().unwrap().len();

    let prepare_control = prepare_control(
        "index-cancel",
        1,
        2,
        reference.clone(),
        config_snapshot.clone(),
        "runtime-index-cancel",
    );
    let reply = controller
        .apply_activation_control(
            prepare_control.clone(),
            &fixture.resolver,
            &config_resolver,
            Some(&mapping_service_db()),
        )
        .await
        .expect("prepare control should return a reply")
        .expect("prepare control should return a terminal reply");
    assert!(matches!(reply, AssemblyActivationControl::Prepared { .. }));

    // M2: Prepare must not disturb the committed generation or provision.
    let health = controller.health().expect("admission health");
    assert!(health.candidate.is_none());
    let current = controller.active().unwrap().unwrap();
    assert!(Arc::ptr_eq(&active, &current));
    assert_eq!(current.generation(), 1);
    assert_eq!(
        provider.provisioned.lock().unwrap().len(),
        provisioned_before,
        "Prepare must not trigger additional DB provisioning"
    );

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
        .expect("abort without pending materialization must be idempotent")
        .is_none());
    assert!(Arc::ptr_eq(&active, &controller.active().unwrap().unwrap()));
}

#[tokio::test]
async fn commit_records_tuple_without_provisioning_and_preserves_loaded_deployments() {
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
    let provisioned_before = provider.provisioned.lock().unwrap().len();

    // Any accidental materialization during Commit would hit the injected
    // provisioning failure and fail the reply; the M2 Commit must not load.
    provider.fail_with("unique index duplicate data");
    let commit = match prepare_control(
        "index-commit",
        1,
        2,
        reference.clone(),
        config_snapshot.clone(),
        "runtime-index-reject",
    ) {
        AssemblyActivationControl::Prepare {
            profile,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
            ..
        } => AssemblyActivationControl::Commit {
            profile,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
            service_db: None,
        },
        _ => unreachable!(),
    };
    let reply = controller
        .apply_activation_control(
            commit,
            &fixture.resolver,
            &config_resolver,
            Some(&mapping_service_db()),
        )
        .await
        .expect("commit control should return a reply")
        .expect("commit control should return a terminal reply");
    assert!(
        matches!(
            reply,
            AssemblyActivationControl::Register {
                generation: 2,
                ..
            }
        ),
        "unexpected commit reply: {reply:?}"
    );
    assert_eq!(
        provider.provisioned.lock().unwrap().len(),
        provisioned_before,
        "Commit must not trigger additional DB provisioning"
    );
    // Committed metadata tracks the tuple; loaded deployments stay untouched.
    assert!(matches!(
        controller.registration().unwrap(),
        Some(AssemblyActivationControl::Register { generation: 2, .. })
    ));
    assert!(Arc::ptr_eq(&active, &controller.active().unwrap().unwrap()));
    assert_eq!(controller.active().unwrap().unwrap().generation(), 1);
    let deployment_build_id = fixture
        .assembly
        .resolved_deployments
        .first()
        .expect("fixture deployment")
        .deployment_artifact_identity
        .as_str();
    assert!(
        controller.is_loaded(deployment_build_id),
        "recovered deployment must stay in the loaded registry"
    );
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
