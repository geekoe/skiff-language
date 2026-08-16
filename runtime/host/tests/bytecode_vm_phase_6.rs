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
    use skiff_runtime_model::{
        request_heap::RequestHeapLimits,
        vm_heap::{HeapDomainId, HeapEpoch, VmHeap, VmHeapError},
        vm_root::{VmRootSource, VmRootVisitor},
        vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot},
    };
    use skiff_runtime_request::{RequestMemoryLedger, RequestVmHeap};
    use skiff_runtime_scheduler::{
        BytecodeControl, BytecodeResumeFailure, BytecodeSchedulerOutcome, BytecodeSchedulerPorts,
        BytecodeUnit, BytecodeUnitControl, ChildHeapCarrier, ChildHeapState, PendingOwnerDraft,
        PendingPublication, PendingRegistry, PendingWake, PendingWakeQueue,
        RequestExecutionContext, RootEscrow, SettleDisposition,
    };
    use skiff_runtime_transport::protocol::{
        ActivationIdentityFrameMetadata, TaskCallerKind, TaskSubmitRequestFrameHeaderV2,
        TaskTargetKind, RUNTIME_FRAME_SCHEMA_VERSION,
    };
    use skiff_runtime_vm::{VmBudget, VmBudgetClosed, VmFiber, VmSemanticCharge};
    use stages::{
        admitted_artifact, link_input, linked_image, published_positive, request_to_terminal,
        scheduler_to_request,
    };
    use std::sync::{Arc, Mutex};

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

    #[test]
    fn interface_remote_table_links_exact_operation() {
        let image = linked_image(
            Capability::InterfaceRemote,
            "interface-remote-table-links-service-operation",
        );
        let remote = image
            .interface_tables()
            .iter()
            .find_map(|table| match table.kind() {
                skiff_runtime_linked_bytecode::LinkedInterfaceTableKind::Remote(remote) => {
                    Some(remote)
                }
                _ => None,
            })
            .expect("remote interface fixture must produce a Remote method table");
        assert!(!remote.methods().is_empty());
        let operation_index = remote.methods()[0]
            .service_operation()
            .expect("remote method row must join its exact service operation");
        let operation = image
            .service_operations()
            .get(operation_index.get() as usize)
            .expect("remote method service operation index must resolve in the consumer image");
        assert_eq!(
            operation.service_requirement_key(),
            remote.service_requirement_key()
        );
        assert_eq!(
            operation.contract_operation_id(),
            remote.methods()[0].contract_operation_id()
        );
        assert_eq!(
            operation.expected_protocol_identity(),
            remote.callee_protocol_identity()
        );
    }

    #[test]
    fn interface_remote_negative_rejected() {
        stages::assert_interface_remote_negative_rejected("interface-remote-negative");
    }
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

    #[tokio::test(flavor = "current_thread")]
    async fn actor_create_host_handoff_pending_resumes_and_terminates() {
        request_to_terminal(Capability::Actor, "actor-create-pending").await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn task_request_routing_carries_exact_activation_identity_fields() {
        let prefix = "task-routing-exact";
        let request_id = "phase-6-task-routing-exact";
        let fixture = published_positive(Capability::Task, prefix);
        let expected_deployment_revision = fixture.deployment.deployment_revision.clone();
        let expected = fixture
            .request_routing_facts(format!("runtime-phase-6-{prefix}"), request_id.to_string());
        let host = host_harness::RuntimeHostHarness::start(prefix, fixture).await;
        let actual = host.request_routing(request_id);

        assert_eq!(actual, expected);
        assert_eq!(actual.assembly_generation, 1);
        assert_eq!(actual.deployment_revision, expected_deployment_revision);
        assert_eq!(
            actual.runtime_replica_id,
            format!("runtime-phase-6-{prefix}")
        );
        assert_eq!(actual.runtime_id, actual.runtime_replica_id);
        assert_eq!(actual.caller_request_id, request_id);
        assert!(actual
            .assembly_identity
            .as_str()
            .starts_with("skiff-runtime-assembly-v3:sha256:"));
        host.close().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn task_host_durable_acceptance_resumes_and_preserves_exact_build() {
        let prefix = "task-acceptance";
        let request_id = "phase-6-task-acceptance";
        let fixture = published_positive(Capability::Task, prefix);
        let expected_build = fixture.deployment.deployment_artifact_identity.clone();
        let expected_service = fixture.deployment.service_id.clone();
        let expected_assembly = fixture.assembly_identity().as_str().to_string();
        let mut host = host_harness::RuntimeHostHarness::start(prefix, fixture).await;
        host.send_http_request(&request_id, "/phase-6/task", "unary", b"7")
            .await;
        let response = host.response(&request_id).await;

        assert_eq!(response.status, 200);
        assert!(
            !response.chunks.is_empty(),
            "durable task acceptance must resume the parent to an observable terminal"
        );
        let accepted = host.accepted_tasks();
        assert_eq!(accepted.len(), 1);
        let submission = &accepted[0];
        assert_eq!(submission.request.caller_kind, TaskCallerKind::Request);
        assert_eq!(submission.request.caller_request_id, request_id);
        assert_eq!(submission.request.target_kind, TaskTargetKind::Function);
        assert_eq!(submission.request.service_id, expected_service);
        assert_eq!(
            submission.request.build_id.as_deref(),
            Some(expected_build.as_str())
        );
        assert_eq!(
            submission.request.activation_identity.assembly_identity,
            expected_assembly
        );
        assert_eq!(submission.response.rpc_id, submission.request.rpc_id);
        assert_eq!(submission.response.status, "submitted");
        assert_eq!(submission.response.task_id, format!("task-{request_id}"));
        assert_eq!(submission.response.request_id, submission.response.task_id);
        assert_eq!(
            submission.response.task_ref.task_id(),
            submission.response.task_id
        );
        assert_eq!(
            submission.response.task_ref.owner(),
            submission.request.service_id
        );
        assert!(
            !submission.payload.is_empty(),
            "durable function task must carry the encoded recoverable payload"
        );
        host.close().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn task_host_duplicate_acceptance_outlives_parent_terminal_and_preserves_payload() {
        let prefix = "task-duplicate";
        let request_id = "phase-6-task-duplicate";
        let fixture = published_positive(Capability::Task, prefix);
        let mut host = host_harness::RuntimeHostHarness::start(prefix, fixture).await;
        host.send_http_request(&request_id, "/phase-6/task", "unary", b"7")
            .await;
        let first = host.response(&request_id).await;
        host.send_http_request(&request_id, "/phase-6/task", "unary", b"7")
            .await;
        let second = host.response(&request_id).await;

        assert_eq!(first.status, 200);
        assert_eq!(second.status, 200);
        assert!(!first.chunks.is_empty());
        assert!(!second.chunks.is_empty());
        let accepted = host.accepted_tasks();
        assert_eq!(
            accepted.len(),
            1,
            "identical duplicate acceptance must not create a second durable task"
        );
        assert_eq!(
            accepted[0].response.task_id,
            format!("task-{request_id}"),
            "late duplicate acceptance after the parent terminal must return the same durable task id"
        );
        assert!(
            !accepted[0].payload.is_empty(),
            "the durable accepted record must preserve the recoverable task payload"
        );
        host.close().await;
    }

    #[test]
    fn task_fake_store_rejects_conflicting_duplicate_task_id() {
        let store = host_harness::FakeTaskStore::default();
        let request = task_submit_request("task-conflict", "parent-1");
        let first = store
            .accept(request.clone(), b"payload-1".to_vec())
            .expect("first acceptance");
        let duplicate = store
            .accept(request.clone(), b"payload-1".to_vec())
            .expect("identical duplicate acceptance");
        assert_eq!(first, duplicate);
        assert_eq!(store.accepted().len(), 1);

        let error = store
            .accept(request, b"payload-2".to_vec())
            .expect_err("conflicting duplicate task id must fail closed");
        assert!(
            error.contains("conflicts"),
            "rejection must be explicit: {error}"
        );
    }

    fn task_submit_request(
        task_id: &str,
        caller_request_id: &str,
    ) -> TaskSubmitRequestFrameHeaderV2 {
        TaskSubmitRequestFrameHeaderV2 {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "task.submit.request".to_string(),
            rpc_id: format!("rpc:{caller_request_id}"),
            runtime_id: "runtime-a".to_string(),
            caller_kind: TaskCallerKind::Request,
            caller_request_id: caller_request_id.to_string(),
            target_kind: TaskTargetKind::Function,
            service_id: "test.skiff/bytecode-vm-phase-6-task".to_string(),
            service_version: "1.0.0".to_string(),
            service_protocol_identity: "test.skiff/bytecode-vm-phase-6-task:1.0.0".to_string(),
            target: "test.skiff/bytecode-vm-phase-6-task:work".to_string(),
            timing: None,
            task_id: Some(task_id.to_string()),
            build_id: Some(format!(
                "skiff-deployment-artifact-v6:sha256:{}",
                "a".repeat(64)
            )),
            activation_identity: ActivationIdentityFrameMetadata {
                assembly_identity: format!("skiff-runtime-assembly-v3:sha256:{}", "a".repeat(64)),
                generation: 1,
                runtime_replica_id: "runtime-a".to_string(),
                deployment_revision: "rev-1".to_string(),
            },
            trace_id: None,
            caller_target: Some("test.skiff/bytecode-vm-phase-6-task:work".to_string()),
            max_queue_wait_ms: None,
            actor_method: None,
        }
    }

    #[test]
    fn containment_disabled_surfaces_fail_closed() {
        stages::assert_containment_rejected("containment-positive");
    }

    #[test]
    fn containment_negative_surfaces_do_not_publish() {
        stages::assert_containment_rejected("containment-negative");
    }

    struct NoopHeap;

    impl VmHeap for NoopHeap {
        fn validate_live(&self, _value: &ValueSlot) -> Result<(), VmHeapError> {
            Ok(())
        }

        fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
            Ok(*source)
        }

        fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
            Ok(*source)
        }

        fn release_snapshot(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
            Ok(())
        }

        fn release_resource(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    struct NoopBudget;

    impl VmBudget for NoopBudget {
        fn before_dispatch(&mut self) -> Result<(), VmBudgetClosed> {
            Ok(())
        }

        fn poll_interrupt(&mut self) -> Result<(), VmBudgetClosed> {
            Ok(())
        }

        fn charge_semantic(&mut self, _charge: VmSemanticCharge<'_>) -> Result<(), VmBudgetClosed> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct NoRoots;

    impl VmRootSource for NoRoots {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingPendingQueue(Mutex<Vec<PendingWake<u64, NoRoots, NoRoots>>>);

    impl PendingWakeQueue<u64, NoRoots, NoRoots> for RecordingPendingQueue {
        fn enqueue(&self, wake: PendingWake<u64, NoRoots, NoRoots>) {
            self.0.lock().unwrap().push(wake);
        }
    }

    struct SyncUnit;

    impl VmRootSource for SyncUnit {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for SyncUnit {
        type ResumeToken = u64;
        type ResumeOutcome = u64;
        type RootResult = u64;
        type ChildInvocation = u64;
        type AdapterInvocation = u64;
        type StreamItem = u64;
        type PendingOperation = u64;

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> BytecodeUnitControl<Self> {
            BytecodeControl::Complete(1)
        }

        fn resume(
            &mut self,
            _resume: u64,
            _outcome: u64,
        ) -> Result<(), BytecodeResumeFailure<u64, u64>> {
            Ok(())
        }
    }

    fn kernel_heap(ledger: &Arc<RequestMemoryLedger>) -> (HeapDomainId, HeapEpoch, RequestVmHeap) {
        let (domain, epoch) = ledger.mint_heap_identity().expect("mint heap identity");
        let heap = RequestVmHeap::with_ledger(
            Arc::clone(ledger),
            domain.get(),
            epoch.get(),
            RequestHeapLimits::default(),
        );
        (domain, epoch, heap)
    }

    #[test]
    fn phase_6_kernel_owner_bundle() {
        let ledger = Arc::new(RequestMemoryLedger::new(1024));
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let (domain, epoch, heap) = kernel_heap(&ledger);
        let carrier = ChildHeapCarrier::new(
            Box::new(heap),
            domain,
            epoch,
            ledger.zero_lease().expect("zero carrier lease"),
            context
                .child_heap_registration()
                .mint_lease()
                .expect("child heap owner lease"),
        );

        assert_eq!(carrier.domain(), domain);
        assert_eq!(carrier.epoch(), epoch);
        assert_eq!(carrier.state(), ChildHeapState::Prepared);

        drop(carrier);
        let snapshot = context.into_not_started();
        assert_eq!(snapshot.child_heap.current, 0);
        assert!(snapshot.child_heap.ever_created);
        assert_eq!(snapshot.child.current, 0);
        assert_eq!(ledger.snapshot().committed, 0);
    }

    #[test]
    fn phase_6_kernel_root_visit() {
        struct CountingVisitor {
            roots: Vec<ValueSlot>,
        }

        impl VmRootVisitor for CountingVisitor {
            fn visit_root(&mut self, root: &ValueSlot) -> Result<(), VmHeapError> {
                self.roots.push(*root);
                Ok(())
            }
        }

        let ledger = Arc::new(RequestMemoryLedger::new(1024));
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let (domain, epoch, mut heap) = kernel_heap(&ledger);
        let slot = heap
            .alloc_typed_string(
                "root".to_string(),
                CompactTypeTag::try_from_type_index(7).expect("type tag"),
                ValueFlags::new(0),
            )
            .expect("allocate root");
        let mut carrier = ChildHeapCarrier::new(
            Box::new(heap),
            domain,
            epoch,
            ledger.zero_lease().expect("zero carrier lease"),
            context
                .child_heap_registration()
                .mint_lease()
                .expect("child heap owner lease"),
        );
        carrier.attach_boundary_registration(context.boundary_registration());
        carrier
            .publish_staging_root(slot)
            .expect("publish staging root");

        let mut visitor = CountingVisitor { roots: Vec::new() };
        carrier
            .visit_roots(&mut visitor)
            .expect("visit child heap roots");
        assert_eq!(visitor.roots.len(), 1);
        assert_eq!(visitor.roots[0].kind(), Some(ValueKind::RequestHeapRef));
        assert_eq!(
            carrier
                .heap()
                .string_value(&visitor.roots[0])
                .expect("read published string root"),
            "root"
        );

        drop(carrier);
        let snapshot = context.into_not_started();
        assert_eq!(snapshot.child_heap.current, 0);
        assert_eq!(snapshot.boundary.current, 0);
        assert!(snapshot.boundary.ever_created);
        assert_eq!(ledger.snapshot().committed, 0);
    }

    #[test]
    fn phase_6_kernel_memory_reserve_release() {
        let ledger = Arc::new(RequestMemoryLedger::new(256));
        let reservation = ledger.reserve(32).expect("reserve");
        assert_eq!(ledger.snapshot().reserved, 32);
        let lease = reservation.commit();
        assert_eq!(ledger.snapshot().committed, 32);

        let (_domain, _epoch, mut heap) = kernel_heap(&ledger);
        let _ = heap
            .alloc_typed_bytes(
                vec![0; 64],
                CompactTypeTag::try_from_type_index(8).expect("type tag"),
                ValueFlags::new(0),
            )
            .expect("allocate bytes");
        assert!(ledger.snapshot().committed > 32);
        assert!(ledger.snapshot().peak_committed > 32);
        drop(heap);
        assert!(ledger.snapshot().committed >= 32);

        lease.release();
        assert_eq!(ledger.snapshot().committed, 0);
        let terminal = ledger.mark_terminal().expect("terminal at zero");
        assert!(terminal.terminal);
        assert_eq!(terminal.peak_total, ledger.snapshot().peak_total);
    }

    #[test]
    fn phase_6_kernel_sync_no_park() {
        let mut context =
            RequestExecutionContext::<SyncUnit>::create(BytecodeSchedulerPorts::default());
        context.install_root(SyncUnit);
        let (outcome, snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);

        assert!(matches!(outcome, Ok(BytecodeSchedulerOutcome::Complete(1))));
        assert_eq!(snapshot.pending.current, 0);
        assert!(!snapshot.pending.ever_created);
        assert_eq!(snapshot.child.current, 0);
        assert_eq!(snapshot.child_heap.current, 0);
        assert_eq!(snapshot.boundary.current, 0);
        assert_eq!(snapshot.actor.current, 0);
    }

    #[test]
    fn phase_6_kernel_actual_pending_chain() {
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let mut context = context;
        let registry = PendingRegistry::<u64, NoRoots, NoRoots>::new(
            context
                .take_pending_registration()
                .expect("pending registration"),
        );
        let completion = registry.begin(RootEscrow::empty()).expect("begin pending");
        let queue = Arc::new(RecordingPendingQueue::default());
        let publication = registry
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(1u64, NoRoots),
                queue.clone(),
            )
            .expect("publish pending owner");
        assert_eq!(publication, PendingPublication::Waiting);

        let disposition = completion.complete(NoRoots);
        assert!(matches!(disposition, SettleDisposition::Enqueued));
        let wake = queue.0.lock().unwrap().pop().expect("claimed wake queued");
        drop(wake);

        let snapshot = context.into_not_started();
        assert_eq!(snapshot.pending.current, 0);
        assert!(snapshot.pending.ever_created);
        assert_eq!(snapshot.child.current, 0);
        assert_eq!(snapshot.child_heap.current, 0);
        assert_eq!(snapshot.boundary.current, 0);
        assert_eq!(snapshot.actor.current, 0);
    }

    #[test]
    fn phase_6_kernel_cleanup_returns_to_zero() {
        let ledger = Arc::new(RequestMemoryLedger::new(1024));
        let mut context =
            RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let (domain, epoch, heap) = kernel_heap(&ledger);
        let mut carrier = ChildHeapCarrier::new(
            Box::new(heap),
            domain,
            epoch,
            ledger.zero_lease().expect("zero carrier lease"),
            context
                .child_heap_registration()
                .mint_lease()
                .expect("child heap owner lease"),
        );
        carrier
            .attach_pending_cleanup(Box::new(|| {}))
            .expect("attach pending cleanup");

        let registry = PendingRegistry::<u64, NoRoots, NoRoots>::new(
            context
                .take_pending_registration()
                .expect("pending registration"),
        );
        let completion = registry.begin(RootEscrow::empty()).expect("begin pending");
        let queue = Arc::new(RecordingPendingQueue::default());
        registry
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(2u64, NoRoots),
                queue.clone(),
            )
            .expect("publish pending owner");
        assert!(matches!(
            completion.complete(NoRoots),
            SettleDisposition::Enqueued
        ));
        drop(queue.0.lock().unwrap().pop().expect("claimed wake"));

        drop(carrier);
        let snapshot = context.into_not_started();
        assert_eq!(snapshot.pending.current, 0);
        assert_eq!(snapshot.child_heap.current, 0);
        assert_eq!(snapshot.boundary.current, 0);
        assert_eq!(snapshot.actor.current, 0);
        assert_eq!(ledger.snapshot().committed, 0);
        let terminal = ledger.mark_terminal().expect("terminal at zero");
        assert!(terminal.terminal);
    }
}
