#[path = "bytecode_vm_phase_6/fixture.rs"]
mod fixture;
#[path = "bytecode_vm_phase_6/host_chain.rs"]
mod host_chain;
#[path = "bytecode_vm_phase_6/host_harness.rs"]
mod host_harness;
#[path = "bytecode_vm_phase_6/stages.rs"]
mod stages;

use fixture::Capability;

#[cfg(test)]
mod tests {
    use super::*;
    use stages::{
        admitted_artifact, linked_image, published_positive, request_to_terminal,
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
        let image = linked_image(capability, prefix);
        assert!(!image.functions().is_empty());
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
