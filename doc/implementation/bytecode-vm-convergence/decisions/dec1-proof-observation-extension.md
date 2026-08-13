# DEC1-O: bounded Phase 1 proof-observation extension

> Status: consistency-corrected with DEC1-B after independent FAIL; delta re-review required before join
>
> Input: MAP1 revision 1 at `34a9a4a8e2c4b563835a484a4eb655a8d22720b0`
>
> Preserved baseline: accepted Phase 0 observer/lifecycle revisions `5b305744` and `0da6e474`
>
> Accounting authority: corrected [`DEC1-B`](./dec1-budget-and-stop-ownership.md) in the same decision stack
>
> Scope: minimum non-verdict frame, local-call, return, budget and cleanup facts only

## Trigger and question

The source-reviewed T-R/V1 candidate reaches the accepted production composition seam, obtains the exact scalar response
`3.0`, and passes both request-boundary negatives. Its expected-red consumer then reports exactly five missing obligations:

1. root and selected helper frame entry;
2. an actually executed `CallLocal` dispatch;
3. selected helper and root normal return;
4. final raw-execution, charged-unit, hard-limit and poll counts; and
5. exact zero Pending/resource/child owner counts at request cleanup.

The five accepted Phase 0 events cannot prove these facts. Inferring them from response `3.0`, source text, an admitted
opcode, a budget counter without the VM event, absence of event names, or object destruction would be a false green. This
activates MAP1's conditional O1 decision.

The question is how to expose only those facts while preserving the Phase 0 observer's one correlation, strict ordinal
ordering, reentrant-safe bounded queue, panic/drop isolation and lack of execution or verdict authority.

## Decision

Extend the existing `BytecodeExecutionEvent` enum and existing `BytecodeExecutionObserver`. Do not add a Phase 1 sink, a
second observer handle, an observation profile selected by a sink, or a test-only enablement flag.

Four variants are added to the enum. They produce at most six additional event instances for one admitted root request: two
`VmFunctionFrameEntered` events, one `VmLocalCallDispatched`, two `VmFunctionReturned` events and one
`VmBudgetAccounted`. `RequestCleanupComplete` remains the single accepted Phase 0 cleanup event and gains three count and
three monotonic creation fields. All new facts use the same `BytecodeExecutionCorrelation`, the same per-request ordinal
sequence and the same `observe` return type `()` and failure semantics as the five Phase 0 events.

The VM facts intentionally describe a fixed observation window rather than a general trace:

- the root VM frame;
- the first successfully executed direct `CallLocal` whose caller is that root frame; and
- the matching callee's normal return and the root's normal return.

The VCP's `helper(2)` is that first root-local callee. Repeated calls, loops, recursion below that selected callee, tail calls
and later callees do not mint more events. The selected window is part of the event semantics, not sampling controlled by a
sink.

This gives useful production facts without allocating a call trace or making event volume proportional to instruction or
call count.

## Typed event contract

All payload integer fields are fixed-width scalars. Checked projection failure omits the observation and therefore fails
evidence; it must not saturate a coordinate or change execution. Serde continues to use the existing `kind`/`payload` shape
and camel-case payload fields.

Conceptually, the model additions are:

```rust
enum VmObservedFrameRole {
    Root,
    FirstRootLocalCallee,
}

struct VmFunctionFrameEntered {
    role: VmObservedFrameRole,
    function_index: u32,
    frame_depth: u32,
    slot_count: u32,
}

struct VmLocalCallDispatched {
    caller_function_index: u32,
    callee_function_index: u32,
    caller_frame_depth: u32,
    callee_frame_depth: u32,
}

struct VmFunctionReturned {
    role: VmObservedFrameRole,
    function_index: u32,
    caller_function_index: Option<u32>,
    remaining_frame_depth: u32,
}

struct VmBudgetAccounted {
    raw_executed_count: u64,
    charged_instruction_count: u64,
    hard_limit: u64,
    poll_count: u64,
}

struct RequestCleanupComplete {
    pending_owner_count: u64,
    pending_owner_ever_created: bool,
    resource_owner_count: u64,
    resource_owner_ever_created: bool,
    child_owner_count: u64,
    child_owner_ever_created: bool,
}
```

`VmObservedFrameRole` is required because the bounded stream is not an all-frames trace. It lets a consumer distinguish an
intentionally selected callee from an incomplete stream without carrying a frame handle.

The payloads contain no argument, local, operand, result or response value; no heap/resource/pending/frame/image handle; no
entry or fiber object; no callback; no timestamp; and no PASS/FAIL or request-result field. In particular,
`VmBudgetAccounted` has no `terminalReason`. The existing `RequestTerminalClaimed` event and external response frame retain
their separate authority.

`RequestCleanupComplete` must lose `Default`; no `new_zero`, defaulted count or observer-side zero synthesis is allowed.

## Event meanings and sole mint points

| Event | Cardinality for one root request | Sole production mint point | Fact established |
| --- | --- | --- | --- |
| `VmFunctionFrameEntered { role: Root, ... }` | zero or one | `VmFiber::start`, after entry/argument validation and after the root frame and its bounded value segment are fully installed, immediately before returning the runnable fiber | the exact pinned root function owns a real depth-1 frame with the reported slot count |
| `VmLocalCallDispatched` | zero or one | the successful state-transition tail of `VmFiber::execute_call_local`, only for the first `CallLocal` executed by the observed root frame, after argument transfer, child segment creation, child-frame push and region-depth push all succeed | the opcode was not merely decoded: caller ownership moved into a real callee frame |
| `VmFunctionFrameEntered { role: FirstRootLocalCallee, ... }` | exactly paired with the selected call | the same successful `execute_call_local` state-transition tail, immediately after `VmLocalCallDispatched` | the selected callee is installed at depth 2 with the reported frame layout |
| `VmFunctionReturned { role: FirstRootLocalCallee, ... }` | zero or one | the non-root success tail of `VmFiber::execute_return`, after the selected child frame/region/value segment is removed, results are transferred, and the root caller is resumed | the selected helper completed by normal `Return`; it is not inferred from a later root result |
| `VmFunctionReturned { role: Root, ... }` | zero or one | the root success tail of `VmFiber::execute_return`, after frames, frame values and regions are cleared and the fiber state is `Terminal`, immediately before returning `DispatchOutcome::Complete` | the root completed by normal `Return`, with no caller and remaining frame depth zero |
| `VmBudgetAccounted` | exactly one for a completion winner whose request budget was activated | the shared `RequestSupervisor` settlement consumer, copying DEC1-B's already frozen `ExecutionSettlement` after the winning `Active -> Completing` transition and immediately before `RequestTerminalClaimed`; observation runs after locks are released | final authoritative attempted-dispatch count, derived charged count, finite limit and trusted poll count |
| extended `RequestCleanupComplete` | unchanged: zero or one, and exactly one after an admitted request's unique terminal/cleanup path | existing `CleanupGuard::observe_cleanup`, copying the immutable owner-inventory snapshot stored in the unique `CleanupPermit`, outside the supervisor lock and after execution/driver, target, supervised-request and route/image request pins are dropped | current and ever-created facts frozen by the request's sole owner inventory before terminal cleanup |

No event is emitted on a failed frame construction, rejected/malformed `CallLocal`, failed return transition or losing
completion/cleanup race. The event call remains after the semantic mutation and cannot make that mutation succeed or fail.

The observer's fixed state gains only enough one-shot coordinates/flags to select and pair the root and first root-local
callee across cloned observer handles. It must use no `Vec`, map, call stack or per-instruction allocation. The selection
state may suppress observation only; no VM branch, error, fuel decision or terminal may read it.

## DEC1-B budget projection

DEC1-O does not define a raw-accounting method, counter or terminal race. DEC1-B is the sole authority for the request-owned
`ExecutionBudget`, `VmBudget::{before_dispatch,poll_interrupt,charge_semantic}`, the VM-private adjacent
`before_dispatch -> dispatch_one exactly once` boundary and immutable `ExecutionSettlement`. L4 must land and pass its
independent review before the overlapping O1 VM/host edits start.

O1 performs only a four-field projection from that frozen DEC1-B settlement:

- `rawExecutedCount` copies DEC1-B's exact count of successful `before_dispatch` calls. Each call atomically increments one
  raw unit immediately before one private `dispatch_one`; an instruction error remains counted and there is no unused tail.
- `chargedInstructionCount` is DEC1-B's external projection derived from the same raw count and therefore equals
  `rawExecutedCount`; O1 neither stores nor computes an independent charge ledger.
- `hardLimit` copies the finite trusted raw limit pinned in the settlement.
- `pollCount` copies the settlement's count of actual trusted polls.

There is no semantic-charge field and no `terminalReason`. Semantic attribution remains internal to DEC1-B and cannot be
folded into raw work; terminal classification remains solely in `RequestTerminalClaimed` plus the external response action.
The event is emitted for every frozen settlement after budget activation, whether the separately observed terminal is
success, VM failure, budget exhaustion or internal stop.

The observer never reads a mutable stats snapshot, VM-local counter, opcode count or cancellation token. It cannot call
`before_dispatch`, insert another raw hook or cause settlement. Observation occurs only after the state mutation it reports;
it neither authorizes nor charges execution.

## Linearizable request-owner inventory

Phase 1's synchronous path must not create or transfer a Pending owner, native resource owner or scheduler child. Reading a
pending registry, then a wake queue, then a resource table and child depth cannot prove that: an owner can move or disappear
between reads and produce a false all-zero snapshot. This decision forbids that sequential-count implementation.

Each admitted request instead owns one private `RequestExecutionOwnerInventory`. It is the lifecycle authority for these
three domains, not an observer-side mirror. Its state is protected by one lock and has one phase (`Open` or `Frozen`) plus,
for each domain, a current count and a monotonic `ever_created` bit. It has no `Default`, public zero constructor, reset or
merge operation. The state is one fixed-size record: it contains no owner list, registry scan, per-event entry or other
allocation whose size can grow with execution. Count increment is checked; overflow rejects the not-yet-installed owner
without installation or saturation. Its only factory is the private admitted-request driver factory named below.

The request driver owns the only non-cloneable freeze permit. Capability objects may retain opaque registration access to
the same inventory state, but that access has no count/snapshot/freeze method and cannot be changed to another request.
Every actual owner constructor must obtain an uncloneable, domain-typed creation guard from that state. Fallible preparation
may happen first. The guard remains locked until it has both changed that domain's `current`/`ever_created` and installed a
non-cloneable `OwnerLease` in the actual carrier; abort drops only the not-yet-installed guard. Consequently construction and
`Open -> Frozen` have one inventory-lock linearization order, with no counted-but-uninstalled or installed-but-uncounted
window.

The three installation sites are literal: `PendingRegistry::begin` while its new cell is inserted, `ResourceRegistry::register`
while its entry becomes live, and `FlatTrampoline::enter_child` while its new `BlockedUnit` is installed. Each uses the
fixed lock order inventory then carrier/container; no path may enter a container lock and then request a creation guard.
A caller cannot construct the carrier directly or supply a lease for the wrong domain or request:

```text
prepare owner privately
  -> lock inventory / reject if Frozen
  -> lock carrier container in the fixed order
  -> increment + set everCreated + install actual owner/OwnerLease
  -> unlock container/inventory
  -> move the same owner/lease between containers
  -> remove/destroy actual owner
  -> release lease under the same inventory lock
```

An `OwnerLease` is validity, not a diagnostic sidecar: no pending cell/wake, resource entry or blocked child owned by this
request is valid without one. It moves with its owner and cannot be cloned or reconstructed. The pending lease starts in
the `PendingCell` created by `PendingRegistry::begin`; cloned `CompletionHandle`s share that cell and do not duplicate the
lease. `Open -> Waiting`, `Open -> Settled`, `Settled -> PendingWake`, and `Waiting -> PendingWake` move the same lease under
the pending cell state transition. They never release or reacquire it, so inventory `current` remains non-zero across every
registry-to-wake handoff. `BytecodeScheduler::resume_from_pending_wake` retains that same leased wake until root restoration
and resume either finish or fail; only then does successful runnable ownership (or failed cleanup) destroy the pending
carrier and release the lease. DEC1-O never sums `PendingRegistry::live_count`, a wake queue length or two locks.
A stale completion handle that no longer carries/reaches the actual cell lease is not an owner and contributes no count.

The resource lease is a private field of the `ResourceEntry` inserted by `ResourceRegistry::register` and leaves only when
`remove_live` removes that entry. `ResourceTable` becomes an opaque inventory-bound type rather than exposing its inner
`Mutex<ResourceRegistry>`; therefore registration can acquire inventory before the registry and must reject an occupied
handle instead of replacing a leased entry. The child lease is a private field of the `BlockedUnit` installed by
`FlatTrampoline::enter_child`; suspension moves that unit unchanged and child completion removes it before lease release.
These are the actual ownership carriers, not shadow records. All constructor paths, including pending pre-completion,
resource replacement/error and child-start failure, must either return the not-yet-installed guard or install exactly one
leased carrier.

Release removes the carrier and its lease from the container, releases that container lock, and only then releases the
lease under the inventory lock. If release and freeze race, both serialize on the inventory lock: release-first yields
`current == 0, ever_created == true`; freeze-first preserves a non-zero frozen count. Creation-first/freeze-second
preserves non-zero; freeze-first rejects creation without installing a carrier. It is impossible for an installed, even
ephemeral, owner to yield both zero and `ever_created == false`.

The canonical Phase 1 construction is stronger than a zero count. Its request driver keeps the Pending registry/wake queue,
resource table and child/stream executor ports physically absent (`None`): no resource table is installed in the request
heap; it constructs no `RequestAdapterExecutor`, `InMemoryWakeQueue` or `VmStreamSupervisor`; and both
`BytecodeSchedulerPorts` entries remain `None`. Scheduler child/adapter/stream/park controls therefore fail before an owner
constructor, and an unexpected `Parked` outcome fails rather than allocating or waiting on a queue. The inventory reaches
terminal as three `NeverCreated` domains. Structural tests must prove that those `Option`s cannot be populated and no owner
constructor can be called without the request inventory guard. Later phases must revisit this containment rather than reuse
the Phase 1 zero proof as owner-lifetime support.

L5 removes the exported `execute_runtime_bytecode_request{_with_ports}` alternate executor instead of adding a parallel
inventory/freeze path; its focused tests migrate to the canonical start/host-driver boundary. There is one production
driver factory and therefore one place that can mint the inventory and freeze permit.

`drive_bytecode_request` mints the inventory and its unique freeze permit after activation but before request execution
starts, passes only inventory-bound registration access into `start_runtime_bytecode_request`, and retains the permit
across the whole drive loop. `DrivenBytecodeRequest` gains one mandatory sealed carrier beside its result and optional
execution object: `NotStarted(actual frozen snapshot)` or `Started(actual frozen snapshot)`. `NotStarted` is valid only when
the canonical driver has structurally established that all three owner-producing ports/carriers are absent, then freezes
the actual inventory before VM execution starts. Start failure uses that path. Once execution starts, normal completion,
run failure and internal terminal use `Started`, frozen by the one explicit driver finish operation. There is no `Default`,
constant zero, `Drop`-minted or missing-execution fallback. Both paths acquire the inventory lock once, change
`Open -> Frozen`, and return the immutable snapshot before any `RequestSupervisor` completion method. Freeze rejects later
creation; lease drops after freeze cannot rewrite the snapshot.

The snapshot is carried with the driven result into the winning supervisor completion, stored in `Completing`, moved into
the unique `CleanupPermit`, and copied unchanged by `CleanupGuard::observe_cleanup` after the execution and request pins are
dropped. A losing completion has no permit or event. Production does not turn non-zero into failure or normalize it after
drop; the Proof/Gate requires, for every domain, `count == 0` and `everCreated == false`.

The six cleanup fields have these exact meanings:

- `pendingOwnerCount`, `resourceOwnerCount` and `childOwnerCount` are the live lease counts at the atomic freeze point; the
  root scheduler unit is not a child.
- `pendingOwnerEverCreated`, `resourceOwnerEverCreated` and `childOwnerEverCreated` are the corresponding monotonic creation
  bits from the same frozen state.

If execution fails before any domain can be installed, the frozen actual inventory still supplies the six facts. The
pre-activation `RevokedByCancel` and `RevokedBySessionStop` outcomes never enter this driver: each maps once to
`StopWithoutResponse`, creates no inventory/budget and emits no terminal or cleanup event. They cannot be represented as
`NotStarted`, resettled or given a duplicate cleanup. There is no synthetic `NotStarted => zero` fallback in the observer or
cleanup permit.

## Cardinality, order and concurrency

The accepted Phase 0 producer maximum is five events. This decision adds at most six and therefore fixes the Phase 1
production maximum at **11 observations per root request**:

```text
2 admission + 1 root frame + 1 first dispatch
+ 1 selected CallLocal + 1 selected callee frame
+ 2 selected/root returns + 1 budget + 1 terminal + 1 cleanup
= 11
```

`OBSERVATION_QUEUE_CAPACITY` remains 16. Add a model assertion that the named production maximum is no greater than the
queue capacity. The five spare slots are queue headroom, not permission for later phases to add events without another
decision and evidence-epoch change.

For the successful Phase 1 scalar fixture, exact ordinals are:

```text
0  DeploymentImageSelected
1  RouteEntryPinned
2  VmFunctionFrameEntered(Root)
3  VmFirstInstructionDispatched(LoadSlot)
4  VmLocalCallDispatched(root -> helper)
5  VmFunctionFrameEntered(FirstRootLocalCallee)
6  VmFunctionReturned(FirstRootLocalCallee)
7  VmFunctionReturned(Root)
8  VmBudgetAccounted
9  RequestTerminalClaimed(Succeeded)
10 RequestCleanupComplete
```

Other requests may omit events whose successful transition never occurred, but they cannot exceed the per-kind maxima. An
error after helper entry, for example, has no helper/root normal-return fact. No later call substitutes for a selected call
that entered and then failed.

One observer is still created for one reserved root wire request. All clones share the correlation, ordinal queue, existing
first-dispatch claim and the fixed Phase 1 selection flags. There is no global ordering across requests. Concurrent root
requests have independent queues and ordinals starting at zero. A cancellation/completion race can enqueue only through the
winning Phase 0 lifecycle owner; callbacks remain outside VM/request-supervisor locks.

The first successful `VmFiber::start` for the request is the observed root. Phase 1 capability containment forbids a child
fiber before acceptance; a later phase that enables child fibers must not reuse these root events without revisiting the
selection contract.

## Failure isolation and dropped telemetry

The Phase 0 observer behavior is unchanged:

- the sink method returns `()`;
- numbering/enqueue remains under the existing observer lock;
- the sole inline drainer invokes the sink outside that lock and in ordinal order;
- sink panic, ordinal exhaustion and queue overflow drop observation only;
- the default host sink continues to serialize a bounded payload and use non-blocking `TelemetryProducer::try_emit`;
- no retry, backpressure, synchronous telemetry flush or fallback event path is added.

There is no `ObservationDropped` event: it could itself be dropped and would create recursive cardinality. Missing, duplicate,
out-of-order or wrong-correlation facts make that evidence unusable but never change the request result, terminal or cleanup.
The in-process VCP recording sink proves delivery by exact total cardinality and contiguous ordinals. If a Gate consumes the
default telemetry projection instead, a producer/telemetry drop likewise makes the exact 11-event receipt fail; it is not a
runtime failure.

Because canonical production can mint only 11 events, it cannot fill the existing 16-slot queue by itself even if one sink
callback is blocked while other request lifecycle threads enqueue. Reentrant or hostile sink-generated events remain
bounded/droppable and are not trusted production facts.

## Production writers and integration write set

The event structs and fixed observation-window state have one model owner; semantic facts remain with their existing
production owners. DEC1-B's complete L4 write set remains authoritative and is not duplicated here. Its overlap with O1 is
explicitly serialized: L4 first changes `runtime/vm/src/{lib,budget,control,error,fiber,limits,statement}.rs` to establish
the exact adjacent `before_dispatch -> dispatch_one` protocol and remove the old budget errors. Only after that reviewed
L4 change may O1 add bounded frame/call/return minting in `fiber.rs` and consume the frozen settlement at the host. O1 may not
edit the `VmBudget` port or raw dispatch order during that second step.

| Owner lane | Exact production writers | Exact allowed production files |
| --- | --- | --- |
| DEC1-B/L4 prerequisite, not an O1 authority | exact per-dispatch/poll/settlement API and VM-private adjacent consumer | `runtime/vm/src/{lib,budget,control,error,fiber,limits,statement}.rs`; remaining files only as listed by DEC1-B |
| O1 model/VM facts | `BytecodeExecutionObserver` fixed claims; `VmFiber::start`; successful tails of `execute_call_local` and `execute_return` | `runtime/model/src/bytecode_execution_observation.rs`; `runtime/vm/src/fiber.rs` |
| O1 budget projection | winning `RequestSupervisor` copies the four fields from the stored DEC1-B settlement; no accounting mutation | `runtime/model/src/bytecode_execution_observation.rs`; `runtime/host/src/host/request_supervisor.rs` |
| L5 owner inventory core | inventory lock/guard/lease/frozen snapshot; leases embedded in actual pending and blocked-child owners | new `runtime/scheduler/src/owner_inventory.rs`; `runtime/scheduler/src/lib.rs`; `runtime/scheduler/src/pending.rs`; `runtime/scheduler/src/trampoline.rs`; `runtime/scheduler/src/bytecode.rs`; `runtime/scheduler/src/stream_driver.rs` |
| L5 request inventory owner | physically absent Phase 1 ports/tables; resource lease; driver freeze and result carrier | `runtime/request/src/vm_heap.rs`; `runtime/request/src/bytecode_ingress.rs`; `runtime/request/src/continuation_handoff.rs`; `runtime/request/src/lib.rs` |
| L5 host cleanup consumer | pass the frozen snapshot through every completion path into the existing cleanup permit and event | `runtime/host/src/host/request_entry/resumable.rs`; `runtime/host/src/host/request_entry/assembly.rs`; `runtime/host/src/host/request_entry/websocket_jsonrpc.rs`; `runtime/host/src/host/request_supervisor.rs` |

No writer may modify `runtime/host/src/host/bytecode_execution_observation.rs` beyond a focused serialization regression:
its generic typed-event projection already owns the production telemetry path. No new public executor, VM constructor,
registry getter, response hook or result-returning observer API is in the write set.

The focused call-site/test updates needed for the hard-cut APIs are limited to
`runtime/scheduler/tests/bytecode_scheduler.rs`, `runtime/request/src/vm_heap/tests.rs`,
`runtime/request/tests/bytecode_request.rs`, `runtime/host/src/host/request_entry/bytecode_http_tests.rs`,
`runtime/host/src/host/request_entry/tests.rs`, the Phase 0/T-R Proof consumers named below, and inline test modules of the
production files above. A newly required path must be added to MAP1 before editing; a compatibility constructor, default
untracked lease or legacy uninstrumented path is not allowed merely to keep an omitted caller compiling.

L4, O1 and L5 overlap in `fiber.rs`, `bytecode_ingress.rs` and `request_supervisor.rs`. MAP1 must join them in the order
DEC1-B/L4 -> owner inventory/L5 -> O1 final projection/mints, with the original owners resolving conflicts. Parallel edits
or an integration-only semantic repair are not implementation strategies. K1/K2 type migration may rename the concrete
image/entry types, but it does not change these event fields or mint semantics.

## Tests and Proof migration

The production writers own only focused non-verdict tests:

- model: exact serde field names, shared-clone one-shot claims, the 11 <= 16 bound, and preservation of the existing
  concurrent/reentrant/panic/overflow tests;
- VM: root plus first root-local callee selection, post-transition order, no event on rejected call/return, repeated/deep local
  calls staying at the fixed maximum, and no raw value/handle in serialized payloads;
- budget projection: exact four-field serialization from a frozen DEC1-B settlement and no mutable-stats/VM-counter read;
  raw accounting and race tests remain solely DEC1-B's obligation;
- inventory: canonical physical absence; live and then released owners; create-versus-freeze and release-versus-freeze
  barriers; pending cell-to-wake transfer racing freeze; post-freeze creation rejection; and proof that every installed
  creation yields either a non-zero frozen current count or `everCreated == true`;
- cleanup: all six real zero/false facts on the synchronous path and a frozen non-zero or ever-created fact in each domain
  being propagated byte-for-byte through terminal, owner drops and cleanup, including sink panic/reentrancy; retain the
  accepted blocked-callback/finalizing-race tests;
- structure: compile-fail/private-constructor proof that no pending cell/wake, resource entry or blocked child can exist
  without an inventory lease, and reverse checks that the Phase 1 driver constructs no pending queue/registry, resource
  table or child/stream port.

The canonical Proof consumer remains candidate `52bfaf32`'s
`runtime/host/src/host/request_entry/phase_1_runtime_proof_support/observations.rs`. When producers join, its owner must:

1. match the typed enum variants rather than discover new facts through serde string names;
2. require the exact 11-event sequence above for the scalar VCP;
3. match root and helper function indices/depths/slot counts and both normal returns;
4. require positive `rawExecutedCount`, equality with derived `chargedInstructionCount`, the exact finite default
   `hardLimit`, and a positive `pollCount`, all copied from one frozen DEC1-B settlement;
5. remove the expected-red `terminalReason == "succeeded"` assertion, because this decision forbids a budget verdict;
6. require all three cleanup counts to be exactly zero, all three `everCreated` bits to be false, and cleanup to remain last;
7. retain zero observations for the pre-admission disabled-route and expired-deadline negatives.

The current Phase 0 regression source asserts a total of five and indexes terminal/cleanup at ordinals 3/4. Historical
accepted Phase 0 evidence remains immutable. In the new Phase 1 evidence epoch, migrate
`runtime/host/src/host/request_entry/phase_0_vcp_tests.rs` to extract the five base variants, require each exactly once with
their original relative order and fields, and allow only the six bounded Phase 1 instances with the roles/cardinalities
specified here. T-R/V1 owns the full 11-event cardinality; the Phase 0 regression must not silently accept arbitrary extra
variants.

G1 records the new event names/fields and maximum in its Phase 1 schema/receipt. It may validate raw evidence but must not
reimplement VM, budget or cleanup semantics in JavaScript. This event/schema change starts a new candidate and evidence
epoch as required by the Phase Contract; there is no compatibility mode for the unpublished language.

## Rejected alternatives

- **A separate optional Phase 1 sink or observer.** It would split cross-layer ordinal order, permit two correlations or
  delivery policies for one request, and make Proof stitch independently droppable streams.
- **Sink-selected observation interest.** A sink response would control which production facts exist and would make VCP
  cardinality test configuration rather than production behavior.
- **Emit every call/frame/return.** Event volume and serialization work would scale with loops and the raw hard limit, and a
  blocked sink could overflow the accepted queue on ordinary production behavior.
- **A terminal summary containing a collected trace.** It requires a per-request trace buffer and moves call/frame truth away
  from the state-transition owners that can mint it.
- **Infer helper execution from source, response or first dispatch.** None establishes an executed `CallLocal`, a child frame
  or normal return.
- **Infer raw execution from decoded opcodes, semantic attribution or an observer-side counter.** None proves the private
  `before_dispatch -> dispatch_one` boundary, and each creates a second accounting authority.
- **Sequentially sum pending registry/wake, resource-table and child-depth state.** A transfer or release can occur between
  locks and manufacture an all-zero snapshot even though an owner existed.
- **Maintain observer-side owner counters or cloneable inventory tokens.** That creates a second ledger with gaps or double
  counts and lets diagnostics outlive or diverge from the actual carrier.
- **Hard-code cleanup zeroes because Phase 1 disables those capabilities.** It proves the allowlist, not the actual request
  ownership state, and would conceal a reachable legacy owner.
- **Put outcome/reason in the budget event.** It duplicates terminal authority and turns an accounting fact into a verdict.
- **Make drops affect request success or synchronously flush telemetry.** Observation would become execution control and
  violate the accepted Phase 0 failure-isolation contract.

## Independent review questions

A reviewer who did not author this decision must answer all of the following before O1/L4/L5 join:

1. Can any loop, recursion, repeated call, resumed segment, cloned observer or future fiber cause more than two frame, one
   local-call, two return, one budget or one cleanup event for one root correlation?
2. Is every VM event after its exact state mutation succeeds, with zero emission from validation/error paths?
3. Does the selected callee return pair with the same first root-local call rather than whichever function returns first?
4. Are all four budget fields copied from one immutable DEC1-B settlement, with charged count derived from exact successful
   `before_dispatch` count and no second O1 accounting API, VM-local counter or ledger?
5. Is budget emission outside supervisor locks, after a frozen final snapshot and before the unique terminal event on every
   activated-budget terminal path?
6. Does each actual pending cell, resource entry and blocked child require the correct uncloneable inventory lease at its
   sole constructor, with no untracked/default construction path?
7. Does pending cell/registry/wake/scheduler transfer preserve the same live lease without a decrement/re-increment gap,
   and do create/release/freeze race tests prove that an ephemeral owner cannot become zero/never-created?
8. Is one immutable current-plus-ever-created snapshot frozen at the driver terminal transition and then propagated
   byte-for-byte through completion, owner drops and cleanup rather than resampled or normalized?
9. Does cleanup retain the accepted `Active -> Completing -> Cleanup` identity and blocked/reentrant callback race behavior?
10. Is the same Phase 0 observer/sink/correlation/ordinal used end to end, with `observe` returning no control data and no new
   public execution/test seam?
11. Can any payload expose a raw language value, response/result, mutable owner, image/entry/fiber handle, callback or verdict?
12. Do source overflow, sink panic and telemetry `try_emit` drops remain evidence failure only, with the production maximum
    mechanically checked against queue capacity?
13. Does the Phase 0 regression still prove its five base facts exactly while T-R/V1 alone owns the full new sequence?
14. Does the implementation touch only the serialized DEC1-B overlap and the exact O1/L5 writers above, with every newly
    discovered constructor returned to MAP1 rather than hidden behind a compatibility path?

## Remaining implementation choices

No shared semantic choice remains. DEC1-O leaves the L4 representation and API wholly to DEC1-B and consumes only its
reviewed four-field frozen settlement. L5 may choose private names/layout for the fixed-size inventory state and snapshot,
but the single lock, uncloneable carrier leases, three literal installation sites, current-plus-ever-created semantics,
freeze point, propagation and write-set above are fixed. If any actual constructor cannot be made lease-bearing, or the
canonical Phase 1 driver cannot keep all three owner-producing capabilities physically absent, L5 is blocked and must
return to Design; it may not add a second ledger, synthesize zeroes, default a token or weaken the Proof.

## Amendment (MAP1 Revision 12; REV1-L5; 2026-08-13)

The typed event contract's six flat cleanup fields are delivered as one nested payload object: the frozen
`RequestExecutionOwnerInventorySnapshot` is carried byte-for-byte as `RequestCleanupComplete { ownerInventory: ... }`.
Serde continues to use the existing `kind`/`payload` envelope and camel-case payload fields. The binding wire shape is:

```json
{
  "kind": "RequestCleanupComplete",
  "payload": {
    "ownerInventory": {
      "pending":  { "current": "<u64>", "everCreated": "<bool>" },
      "resource": { "current": "<u64>", "everCreated": "<bool>" },
      "child":    { "current": "<u64>", "everCreated": "<bool>" }
    }
  }
}
```

This is the L5-chosen snapshot layout authorized above ("L5 may choose private names/layout for the fixed-size inventory
state and snapshot") and is the binding wire contract for O1 production, T-R typed matching and the Gate schema. The six
facts map 1:1: `pendingOwnerCount -> ownerInventory.pending.current`,
`pendingOwnerEverCreated -> ownerInventory.pending.everCreated`, and likewise for `resource` and `child`. No flat
`pendingOwnerCount`/`resourceOwnerCount`/`childOwnerCount` top-level field is emitted.
