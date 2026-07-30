use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use skiff_artifact_model::{
    AssemblyActivationControl, AssemblyActivationRejectReason, RuntimeAssemblyRef,
    RuntimeConfigSnapshotRef,
};
use skiff_runtime_capability_context::{
    DbCapabilityError, DbCapabilityFuture, DbCapabilityResult, DbCapabilitySource,
    DbProviderBuildInput, DbProviderFactory, DbProviderSource,
};

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

fn prepare_control(
    activation_id: &str,
    expected_generation: u64,
    candidate_generation: u64,
    assembly: RuntimeAssemblyRef,
    config_snapshot: RuntimeConfigSnapshotRef,
    replica_id: &str,
) -> AssemblyActivationControl {
    AssemblyActivationControl::Prepare {
        environment: "fixture".to_string(),
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
