#[path = "bytecode_vm_phase_6/fixture.rs"]
mod fixture;
#[path = "bytecode_vm_phase_6/host_chain.rs"]
mod host_chain;
#[path = "bytecode_vm_phase_6/host_harness.rs"]
mod host_harness;
#[path = "bytecode_vm_phase_6/recoverable_codec.rs"]
mod recoverable_codec;
#[path = "bytecode_vm_phase_6/stages.rs"]
mod stages;

use fixture::Capability;

#[cfg(test)]
mod tests {
    use super::*;
    use stages::{
        admitted_artifact, link_input, linked_image, published_positive, request_to_terminal,
        scheduler_to_request,
    };

    fn assert_stage_one(capability: Capability, prefix: &str) {
        let fixture = published_positive(capability, prefix);
        assert!(!fixture.package_artifact().bytecode.is_none());
    }

    fn assert_stage_two(capability: Capability, prefix: &str) {
        let bytecode = admitted_artifact(capability, prefix);
        assert!(!bytecode.view().functions().is_empty());
    }

    fn assert_stage_three(capability: Capability, prefix: &str) {
        let hydrated = link_input(capability, prefix);
        assert!(
            hydrated
                .packages()
                .values()
                .any(|package| package.has_bytecode()),
            "production loader did not expose bytecode-bearing packages at atomic-link input"
        );
    }

    fn assert_stage_four(capability: Capability, prefix: &str) {
        let image = linked_image(capability, prefix);
        assert!(!image.functions().is_empty());
    }

    macro_rules! capability_matrix {
        ($capability:expr, $prefix:literal, $s1:ident, $s2:ident, $s3:ident, $s4:ident, $s5:ident, $s6:ident) => {
            #[test]
            fn $s1() {
                assert_stage_one($capability, $prefix);
            }
            #[test]
            fn $s2() {
                assert_stage_two($capability, $prefix);
            }
            #[test]
            fn $s3() {
                assert_stage_three($capability, $prefix);
            }
            #[test]
            fn $s4() {
                assert_stage_four($capability, $prefix);
            }
            #[tokio::test(flavor = "current_thread")]
            async fn $s5() {
                scheduler_to_request($capability, $prefix).await;
            }
            #[tokio::test(flavor = "current_thread")]
            async fn $s6() {
                request_to_terminal($capability, $prefix).await;
            }
        };
    }

    // Each S1-S6 test intentionally uses the real production compiler,
    // publication store, atomic linker, RuntimeHost request entry, and
    // terminal path. On the current accepted main these capability fixtures
    // are rejected before publication, so every matrix command is a genuine
    // expected-red proof until its Phase 6 lane lands.
    capability_matrix!(
        Capability::Service,
        "service-s1",
        service_s1,
        service_s2,
        service_s3,
        service_s4,
        service_s5,
        service_s6
    );
    capability_matrix!(
        Capability::InterfaceLocal,
        "interface-local-s1",
        interface_local_s1,
        interface_local_s2,
        interface_local_s3,
        interface_local_s4,
        interface_local_s5,
        interface_local_s6
    );

    #[tokio::test(flavor = "current_thread")]
    async fn interface_local_unary_table_success() {
        let image = linked_image(
            Capability::InterfaceLocal,
            "interface-local-unary-table-success",
        );
        let table = image
            .interface_tables()
            .iter()
            .find_map(|table| match table.kind() {
                skiff_runtime_linked_bytecode::LinkedInterfaceTableKind::Local(local) => {
                    Some(local)
                }
                _ => None,
            })
            .expect("local interface fixture must produce a local method table");
        assert_eq!(table.methods().len(), 2);
        assert_eq!(table.methods()[0].method_slot(), 0);
        assert_eq!(table.methods()[1].method_slot(), 1);

        let response = host_chain::interface_local_named_to_terminal(
            "interface-local-success",
            "test.skiff/bytecode-vm-phase-6-interface-local",
            "interface-local-unary-table-success-request",
        )
        .await;
        assert_eq!(response.status, 200);
        assert!(!response.chunks.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn interface_local_unary_table_throw() {
        let error = host_chain::interface_local_named_throw_terminal(
            "interface-local-throw",
            "test.skiff/bytecode-vm-phase-6-interface-local-throw",
            "interface-local-unary-table-throw",
        )
        .await;
        assert!(!error.code.is_empty());
        assert!(!error.message.is_empty());
        assert!(
            error
                .status
                .map_or(true, |status| (400..=599).contains(&status)),
            "ordinary interface-local throw must map to an HTTP error terminal"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn interface_local_unary_table_pending() {
        let response = host_chain::interface_local_named_to_terminal(
            "interface-local-pending",
            "test.skiff/bytecode-vm-phase-6-interface-local-pending",
            "interface-local-unary-table-pending",
        )
        .await;
        assert_eq!(response.status, 200);
        assert!(!response.chunks.is_empty());
    }

    #[test]
    fn interface_local_bad_slot_rejected() {
        stages::assert_interface_local_named_rejected(
            "interface-local-bad-slot",
            "test.skiff/bytecode-vm-phase-6-interface-local-bad-slot",
            "interface-local-bad-slot",
        );
    }

    #[test]
    fn interface_local_bad_carrier_rejected() {
        stages::assert_interface_local_named_rejected(
            "interface-local-bad-carrier",
            "test.skiff/bytecode-vm-phase-6-interface-local-bad-carrier",
            "interface-local-bad-carrier",
        );
    }

    #[test]
    fn interface_local_bad_signature_rejected() {
        stages::assert_interface_local_named_rejected(
            "interface-local-bad-signature",
            "test.skiff/bytecode-vm-phase-6-interface-local-bad-signature",
            "interface-local-bad-signature",
        );
    }
    capability_matrix!(
        Capability::InterfaceRemote,
        "interface-remote-s1",
        interface_remote_s1,
        interface_remote_s2,
        interface_remote_s3,
        interface_remote_s4,
        interface_remote_s5,
        interface_remote_s6
    );
    capability_matrix!(
        Capability::Callback,
        "callback-s1",
        callback_s1,
        callback_s2,
        callback_s3,
        callback_s4,
        callback_s5,
        callback_s6
    );

    #[test]
    fn callback_negative_rejected() {
        stages::assert_callback_negative_rejected("callback-negative");
    }

    capability_matrix!(
        Capability::Recoverable,
        "recoverable-s1",
        recoverable_s1,
        recoverable_s2,
        recoverable_s3,
        recoverable_s4,
        recoverable_s5,
        recoverable_s6
    );
    capability_matrix!(
        Capability::Db,
        "db-s1",
        db_s1,
        db_s2,
        db_s3,
        db_s4,
        db_s5,
        db_s6
    );
    capability_matrix!(
        Capability::Task,
        "task-s1",
        task_s1,
        task_s2,
        task_s3,
        task_s4,
        task_s5,
        task_s6
    );
    capability_matrix!(
        Capability::Actor,
        "actor-s1",
        actor_s1,
        actor_s2,
        actor_s3,
        actor_s4,
        actor_s5,
        actor_s6
    );

    #[test]
    fn containment_disabled_surfaces_fail_closed() {
        stages::assert_containment_rejected("containment-positive");
    }

    #[test]
    fn containment_negative_surfaces_do_not_publish() {
        stages::assert_containment_rejected("containment-negative");
    }

    #[test]
    fn phase_6_kernel_owner_bundle() {
        published_positive(Capability::Service, "kernel-owner-bundle");
    }

    #[test]
    fn phase_6_kernel_root_visit() {
        published_positive(Capability::InterfaceLocal, "kernel-root-visit");
    }

    #[test]
    fn phase_6_kernel_memory_reserve_release() {
        published_positive(Capability::Callback, "kernel-memory");
    }

    #[test]
    fn phase_6_kernel_sync_no_park() {
        published_positive(Capability::Db, "kernel-sync-no-park");
    }

    #[test]
    fn phase_6_kernel_actual_pending_chain() {
        published_positive(Capability::Task, "kernel-pending-chain");
    }

    #[test]
    fn phase_6_kernel_cleanup_returns_to_zero() {
        published_positive(Capability::Actor, "kernel-cleanup");
    }
}
