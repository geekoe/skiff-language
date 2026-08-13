use std::{
    collections::VecDeque,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Mutex},
};

use serde::Serialize;
use skiff_artifact_model::{
    ContractOperationId, DeploymentArtifactIdentity, GatewayEntryIdentity, GatewayEntryKey, Opcode,
    IngressSelector, ServiceDeploymentRef,
};

const OBSERVATION_QUEUE_CAPACITY: usize = 16;

/// Phase 1 production maximum: eleven observations per admitted root request.
///
/// The five spare queue slots are headroom, not permission to mint further
/// events without another decision and evidence-epoch change.
pub const PHASE_1_PRODUCTION_OBSERVATION_MAX: usize = 11;

const _: () = assert!(
    PHASE_1_PRODUCTION_OBSERVATION_MAX <= OBSERVATION_QUEUE_CAPACITY,
    "Phase 1 production observation maximum must fit the bounded queue"
);

/// Wire-ingress identity shared by every observation for one bytecode request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BytecodeExecutionCorrelation {
    pub router_session_id: String,
    pub request_id: String,
}

/// One ordered, read-only fact about a production bytecode request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BytecodeExecutionObservation {
    pub correlation: BytecodeExecutionCorrelation,
    pub ordinal: u64,
    pub event: BytecodeExecutionEvent,
}

/// Failure-isolated consumer of bytecode execution observations.
///
/// Implementations cannot return control data to the runtime. The observer
/// isolates panics and reentrant calls and never calls a sink while holding its
/// state lock. Because this is a synchronous trait, an arbitrary sink can
/// still block its one inline drainer indefinitely. Production's default sink
/// therefore uses only try-lock configuration and queue operations.
pub trait BytecodeExecutionEventSink: Send + Sync + 'static {
    fn observe(&self, observation: BytecodeExecutionObservation);
}

#[derive(Default)]
struct NoopBytecodeExecutionEventSink;

impl BytecodeExecutionEventSink for NoopBytecodeExecutionEventSink {
    fn observe(&self, _observation: BytecodeExecutionObservation) {}
}

struct BytecodeExecutionObserverState {
    next: u64,
    queue: VecDeque<BytecodeExecutionObservation>,
    draining: bool,
    dispatch_claimed: bool,
    root_frame_entry_claimed: bool,
    root_local_call_claimed: bool,
    first_root_local_callee_frame_entry_claimed: bool,
    first_root_local_callee_return_claimed: bool,
    root_return_claimed: bool,
}

/// Cloneable per-request handle carrying one correlation and shared ordering.
#[derive(Clone)]
pub struct BytecodeExecutionObserver {
    sink: Arc<dyn BytecodeExecutionEventSink>,
    correlation: Arc<BytecodeExecutionCorrelation>,
    state: Arc<Mutex<BytecodeExecutionObserverState>>,
}

impl std::fmt::Debug for BytecodeExecutionObserver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BytecodeExecutionObserver")
            .field("correlation", &self.correlation)
            .finish_non_exhaustive()
    }
}

impl BytecodeExecutionObserver {
    pub fn new(
        sink: Arc<dyn BytecodeExecutionEventSink>,
        correlation: BytecodeExecutionCorrelation,
    ) -> Self {
        Self {
            sink,
            correlation: Arc::new(correlation),
            state: Arc::new(Mutex::new(BytecodeExecutionObserverState {
                next: 0,
                queue: VecDeque::with_capacity(OBSERVATION_QUEUE_CAPACITY),
                draining: false,
                dispatch_claimed: false,
                root_frame_entry_claimed: false,
                root_local_call_claimed: false,
                first_root_local_callee_frame_entry_claimed: false,
                first_root_local_callee_return_claimed: false,
                root_return_claimed: false,
            })),
        }
    }

    pub fn noop(correlation: BytecodeExecutionCorrelation) -> Self {
        Self::new(Arc::new(NoopBytecodeExecutionEventSink), correlation)
    }

    pub fn correlation(&self) -> &BytecodeExecutionCorrelation {
        self.correlation.as_ref()
    }

    /// Emits one fact without exposing any result or control authority.
    ///
    /// Ordinal exhaustion, bounded-queue overflow and sink panics are treated
    /// as observation drops. Numbering and enqueue happen under one lock; the
    /// sole inline drainer delivers outside the lock in strict ordinal order.
    pub fn observe(&self, event: BytecodeExecutionEvent) {
        let owns_drain = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.queue.len() >= OBSERVATION_QUEUE_CAPACITY {
                return;
            }
            let Some(next) = state.next.checked_add(1) else {
                return;
            };
            let observation = BytecodeExecutionObservation {
                correlation: self.correlation.as_ref().clone(),
                ordinal: state.next,
                event,
            };
            state.next = next;
            state.queue.push_back(observation);
            if state.draining {
                false
            } else {
                state.draining = true;
                true
            }
        };
        if !owns_drain {
            return;
        }

        loop {
            let observation = {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                let Some(observation) = state.queue.pop_front() else {
                    state.draining = false;
                    return;
                };
                observation
            };
            let _ = catch_unwind(AssertUnwindSafe(|| self.sink.observe(observation)));
        }
    }

    /// Claims the successful-dispatch marker at most once across every fiber
    /// derived from this root request. The successful opcode owner remains the
    /// sole production event mint point.
    pub fn claim_vm_first_instruction_dispatch(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.dispatch_claimed {
            false
        } else {
            state.dispatch_claimed = true;
            true
        }
    }

    /// Claims the observed root-frame-entry marker at most once across every
    /// handle sharing this correlation. Selection state suppresses observation
    /// only and never participates in execution decisions.
    pub fn claim_root_frame_entry(&self) -> bool {
        self.claim_selection_flag(|state| &mut state.root_frame_entry_claimed)
    }

    /// Claims the first observed root-local call marker at most once.
    pub fn claim_root_local_call(&self) -> bool {
        self.claim_selection_flag(|state| &mut state.root_local_call_claimed)
    }

    /// Claims the first root-local callee frame-entry marker at most once.
    pub fn claim_first_root_local_callee_frame_entry(&self) -> bool {
        self.claim_selection_flag(|state| {
            &mut state.first_root_local_callee_frame_entry_claimed
        })
    }

    /// Claims the first root-local callee normal-return marker at most once.
    pub fn claim_first_root_local_callee_return(&self) -> bool {
        self.claim_selection_flag(|state| {
            &mut state.first_root_local_callee_return_claimed
        })
    }

    /// Claims the observed root normal-return marker at most once.
    pub fn claim_root_return(&self) -> bool {
        self.claim_selection_flag(|state| &mut state.root_return_claimed)
    }

    fn claim_selection_flag(
        &self,
        select: impl FnOnce(&mut BytecodeExecutionObserverState) -> &mut bool,
    ) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let flag = select(&mut state);
        if *flag {
            false
        } else {
            *flag = true;
            true
        }
    }
}

/// Canonical typed event shape projected by the host telemetry sink.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "payload")]
pub enum BytecodeExecutionEvent {
    #[serde(rename = "DeploymentImageSelected")]
    DeploymentImageSelected(DeploymentImageSelected),
    #[serde(rename = "RouteEntryPinned")]
    RouteEntryPinned(RouteEntryPinned),
    #[serde(rename = "VmFirstInstructionDispatched")]
    VmFirstInstructionDispatched(VmFirstInstructionDispatched),
    #[serde(rename = "VmFunctionFrameEntered")]
    VmFunctionFrameEntered(VmFunctionFrameEntered),
    #[serde(rename = "VmLocalCallDispatched")]
    VmLocalCallDispatched(VmLocalCallDispatched),
    #[serde(rename = "VmFunctionReturned")]
    VmFunctionReturned(VmFunctionReturned),
    #[serde(rename = "VmBudgetAccounted")]
    VmBudgetAccounted(VmBudgetAccounted),
    #[serde(rename = "RequestTerminalClaimed")]
    RequestTerminalClaimed(RequestTerminalClaimed),
    #[serde(rename = "RequestCleanupComplete")]
    RequestCleanupComplete(RequestCleanupComplete),
}

/// Role of one observed frame inside the bounded Phase 1 observation window.
///
/// The role disambiguates the intentionally selected first root-local callee
/// from the root without carrying a frame handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum VmObservedFrameRole {
    Root,
    FirstRootLocalCallee,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentImageSelected {
    pub deployment: ServiceDeploymentRef,
    pub deployment_build_id: DeploymentArtifactIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteEntryPinned {
    pub image_owner: ServiceDeploymentRef,
    pub selector: BytecodeRouteEntrySelector,
    pub gateway_key: Option<GatewayEntryKey>,
    pub gateway_identity: Option<GatewayEntryIdentity>,
    pub callable_role: Option<BytecodeGatewayCallableRole>,
    pub verified_function_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "identity", rename_all = "camelCase")]
pub enum BytecodeRouteEntrySelector {
    Operation(ContractOperationId),
    Gateway(IngressSelector),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum BytecodeGatewayCallableRole {
    Handler,
    Pre,
    Guard,
    CloseHandler,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VmFirstInstructionDispatched {
    pub image_owner: ServiceDeploymentRef,
    pub root_entry_function_index: u32,
    pub current_function_index: u32,
    pub instruction_index: u32,
    pub opcode: Opcode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VmFunctionFrameEntered {
    pub role: VmObservedFrameRole,
    pub function_index: u32,
    pub frame_depth: u32,
    pub slot_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VmLocalCallDispatched {
    pub caller_function_index: u32,
    pub callee_function_index: u32,
    pub caller_frame_depth: u32,
    pub callee_frame_depth: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VmFunctionReturned {
    pub role: VmObservedFrameRole,
    pub function_index: u32,
    pub caller_function_index: Option<u32>,
    pub remaining_frame_depth: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VmBudgetAccounted {
    pub raw_executed_count: u64,
    pub charged_instruction_count: u64,
    pub hard_limit: u64,
    pub poll_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum BytecodeRequestTerminal {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestTerminalClaimed {
    pub terminal: BytecodeRequestTerminal,
}

/// One frozen, immutable fact about a single request owner domain.
///
/// `current` counts owners that are still live at freeze time; `ever_created`
/// records whether the domain ever minted an owner, even one that has since
/// been released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenOwnerDomain {
    pub current: u64,
    pub ever_created: bool,
}

/// The actual pending/resource/child owner inventory frozen for one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestExecutionOwnerInventorySnapshot {
    pub pending: FrozenOwnerDomain,
    pub resource: FrozenOwnerDomain,
    pub child: FrozenOwnerDomain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestCleanupComplete {
    pub owner_inventory: RequestExecutionOwnerInventorySnapshot,
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Barrier, Mutex,
        },
        thread,
    };

    use super::*;

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<BytecodeExecutionObservation>>);

    impl BytecodeExecutionEventSink for RecordingSink {
        fn observe(&self, observation: BytecodeExecutionObservation) {
            self.0.lock().unwrap().push(observation);
        }
    }

    fn correlation() -> BytecodeExecutionCorrelation {
        BytecodeExecutionCorrelation {
            router_session_id: "session".to_string(),
            request_id: "request".to_string(),
        }
    }

    fn terminal() -> BytecodeExecutionEvent {
        BytecodeExecutionEvent::RequestTerminalClaimed(RequestTerminalClaimed {
            terminal: BytecodeRequestTerminal::Succeeded,
        })
    }

    fn dispatch() -> VmFirstInstructionDispatched {
        VmFirstInstructionDispatched {
            image_owner: ServiceDeploymentRef {
                service_id: "example.com/service".to_string(),
                contract_version: "1.0.0".to_string(),
                deployment_revision: "revision".into(),
                deployment_artifact_identity: "deployment".into(),
            },
            root_entry_function_index: 1,
            current_function_index: 1,
            instruction_index: 0,
            opcode: Opcode::Const,
        }
    }

    #[test]
    fn concurrent_barrier_delivery_is_strictly_ordinal() {
        const CONCURRENCY: usize = 12;
        let sink = Arc::new(RecordingSink::default());
        let observer = BytecodeExecutionObserver::new(sink.clone(), correlation());
        let barrier = Arc::new(Barrier::new(CONCURRENCY));
        let threads = (0..CONCURRENCY)
            .map(|_| {
                let observer = observer.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    observer.observe(terminal());
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }

        let records = sink.0.lock().unwrap();
        assert_eq!(
            records
                .iter()
                .map(|record| record.ordinal)
                .collect::<Vec<_>>(),
            (0..CONCURRENCY as u64).collect::<Vec<_>>()
        );
        assert!(records
            .iter()
            .all(|record| record.correlation == correlation()));
    }

    struct ReentrantPanickingSink {
        records: Mutex<Vec<BytecodeExecutionObservation>>,
        observer: Mutex<Option<BytecodeExecutionObserver>>,
        reentered: AtomicBool,
    }

    impl BytecodeExecutionEventSink for ReentrantPanickingSink {
        fn observe(&self, observation: BytecodeExecutionObservation) {
            self.records.lock().unwrap().push(observation);
            if !self.reentered.swap(true, Ordering::SeqCst) {
                self.observer
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .observe(terminal());
                panic!("sink failure after reentrant enqueue");
            }
        }
    }

    #[test]
    fn reentrant_observation_is_queued_and_panic_does_not_stop_drain() {
        let sink = Arc::new(ReentrantPanickingSink {
            records: Mutex::new(Vec::new()),
            observer: Mutex::new(None),
            reentered: AtomicBool::new(false),
        });
        let observer = BytecodeExecutionObserver::new(sink.clone(), correlation());
        *sink.observer.lock().unwrap() = Some(observer.clone());

        observer.observe(terminal());

        assert_eq!(
            sink.records
                .lock()
                .unwrap()
                .iter()
                .map(|record| record.ordinal)
                .collect::<Vec<_>>(),
            [0, 1]
        );
    }

    struct BlockingFirstSink {
        records: Mutex<Vec<BytecodeExecutionObservation>>,
        entered: Arc<Barrier>,
        release: Arc<Barrier>,
        first: AtomicBool,
    }

    impl BytecodeExecutionEventSink for BlockingFirstSink {
        fn observe(&self, observation: BytecodeExecutionObservation) {
            self.records.lock().unwrap().push(observation);
            if self.first.swap(false, Ordering::SeqCst) {
                self.entered.wait();
                self.release.wait();
            }
        }
    }

    #[test]
    fn reentrant_queue_is_bounded_and_overflow_is_dropped() {
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let sink = Arc::new(BlockingFirstSink {
            records: Mutex::new(Vec::new()),
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
            first: AtomicBool::new(true),
        });
        let observer = BytecodeExecutionObserver::new(sink.clone(), correlation());
        let drainer = {
            let observer = observer.clone();
            thread::spawn(move || observer.observe(terminal()))
        };
        entered.wait();
        for _ in 0..=OBSERVATION_QUEUE_CAPACITY {
            observer.observe(terminal());
        }
        release.wait();
        drainer.join().unwrap();
        observer.observe(terminal());

        let ordinals = sink
            .records
            .lock()
            .unwrap()
            .iter()
            .map(|record| record.ordinal)
            .collect::<Vec<_>>();
        assert_eq!(
            ordinals,
            (0..=(OBSERVATION_QUEUE_CAPACITY as u64 + 1)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn clones_share_one_root_dispatch_marker() {
        let sink = Arc::new(RecordingSink::default());
        let observer = BytecodeExecutionObserver::new(sink.clone(), correlation());
        for candidate in [observer.clone(), observer.clone()] {
            if candidate.claim_vm_first_instruction_dispatch() {
                candidate.observe(BytecodeExecutionEvent::VmFirstInstructionDispatched(
                    dispatch(),
                ));
            }
        }

        let records = sink.0.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert!(matches!(
            &records[0].event,
            BytecodeExecutionEvent::VmFirstInstructionDispatched(_)
        ));
    }

    #[test]
    fn new_event_payloads_serialize_exact_field_names_without_extra_keys() {
        assert_eq!(
            serde_json::to_value(BytecodeExecutionEvent::VmFunctionFrameEntered(
                VmFunctionFrameEntered {
                    role: VmObservedFrameRole::Root,
                    function_index: 3,
                    frame_depth: 1,
                    slot_count: 2,
                }
            ))
            .unwrap(),
            serde_json::json!({
                "kind": "VmFunctionFrameEntered",
                "payload": {
                    "role": "Root",
                    "functionIndex": 3,
                    "frameDepth": 1,
                    "slotCount": 2,
                }
            })
        );
        assert_eq!(
            serde_json::to_value(BytecodeExecutionEvent::VmFunctionFrameEntered(
                VmFunctionFrameEntered {
                    role: VmObservedFrameRole::FirstRootLocalCallee,
                    function_index: 7,
                    frame_depth: 2,
                    slot_count: 4,
                }
            ))
            .unwrap(),
            serde_json::json!({
                "kind": "VmFunctionFrameEntered",
                "payload": {
                    "role": "FirstRootLocalCallee",
                    "functionIndex": 7,
                    "frameDepth": 2,
                    "slotCount": 4,
                }
            })
        );
        assert_eq!(
            serde_json::to_value(BytecodeExecutionEvent::VmLocalCallDispatched(
                VmLocalCallDispatched {
                    caller_function_index: 3,
                    callee_function_index: 7,
                    caller_frame_depth: 1,
                    callee_frame_depth: 2,
                }
            ))
            .unwrap(),
            serde_json::json!({
                "kind": "VmLocalCallDispatched",
                "payload": {
                    "callerFunctionIndex": 3,
                    "calleeFunctionIndex": 7,
                    "callerFrameDepth": 1,
                    "calleeFrameDepth": 2,
                }
            })
        );
        assert_eq!(
            serde_json::to_value(BytecodeExecutionEvent::VmFunctionReturned(
                VmFunctionReturned {
                    role: VmObservedFrameRole::FirstRootLocalCallee,
                    function_index: 7,
                    caller_function_index: Some(3),
                    remaining_frame_depth: 1,
                }
            ))
            .unwrap(),
            serde_json::json!({
                "kind": "VmFunctionReturned",
                "payload": {
                    "role": "FirstRootLocalCallee",
                    "functionIndex": 7,
                    "callerFunctionIndex": 3,
                    "remainingFrameDepth": 1,
                }
            })
        );
        assert_eq!(
            serde_json::to_value(BytecodeExecutionEvent::VmFunctionReturned(
                VmFunctionReturned {
                    role: VmObservedFrameRole::Root,
                    function_index: 3,
                    caller_function_index: None,
                    remaining_frame_depth: 0,
                }
            ))
            .unwrap(),
            serde_json::json!({
                "kind": "VmFunctionReturned",
                "payload": {
                    "role": "Root",
                    "functionIndex": 3,
                    "callerFunctionIndex": null,
                    "remainingFrameDepth": 0,
                }
            })
        );
        assert_eq!(
            serde_json::to_value(BytecodeExecutionEvent::VmBudgetAccounted(
                VmBudgetAccounted {
                    raw_executed_count: 11,
                    charged_instruction_count: 11,
                    hard_limit: 100_000,
                    poll_count: 2,
                }
            ))
            .unwrap(),
            serde_json::json!({
                "kind": "VmBudgetAccounted",
                "payload": {
                    "rawExecutedCount": 11,
                    "chargedInstructionCount": 11,
                    "hardLimit": 100000,
                    "pollCount": 2,
                }
            })
        );
    }

    #[test]
    fn nested_cleanup_payload_serializes_the_adjudicated_wire_shape() {
        let event = BytecodeExecutionEvent::RequestCleanupComplete(RequestCleanupComplete {
            owner_inventory: RequestExecutionOwnerInventorySnapshot {
                pending: FrozenOwnerDomain {
                    current: 1,
                    ever_created: true,
                },
                resource: FrozenOwnerDomain {
                    current: 0,
                    ever_created: false,
                },
                child: FrozenOwnerDomain {
                    current: 2,
                    ever_created: true,
                },
            },
        });
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            serde_json::json!({
                "kind": "RequestCleanupComplete",
                "payload": {
                    "ownerInventory": {
                        "pending": { "current": 1, "everCreated": true },
                        "resource": { "current": 0, "everCreated": false },
                        "child": { "current": 2, "everCreated": true },
                    }
                }
            })
        );
    }

    #[test]
    fn each_phase_1_selection_claim_succeeds_exactly_once_across_clones() {
        let observer = BytecodeExecutionObserver::new(
            Arc::new(RecordingSink::default()),
            correlation(),
        );
        let clone = observer.clone();

        assert!(observer.claim_root_frame_entry());
        assert!(!clone.claim_root_frame_entry());
        assert!(observer.claim_root_local_call());
        assert!(!clone.claim_root_local_call());
        assert!(observer.claim_first_root_local_callee_frame_entry());
        assert!(!clone.claim_first_root_local_callee_frame_entry());
        assert!(observer.claim_first_root_local_callee_return());
        assert!(!clone.claim_first_root_local_callee_return());
        assert!(observer.claim_root_return());
        assert!(!clone.claim_root_return());
    }

    #[test]
    fn phase_1_production_observation_max_fits_the_bounded_queue() {
        assert!(PHASE_1_PRODUCTION_OBSERVATION_MAX <= OBSERVATION_QUEUE_CAPACITY);
    }
}
