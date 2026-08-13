# DEC1-B: raw budget and internal-stop ownership

> Status: consistency-corrected after independent FAIL; delta review required before L4/L5
>
> Input: MAP1 revision 1 at `34a9a4a8e2c4b563835a484a4eb655a8d22720b0`
>
> Cleanup join: [`DEC1-O`](./dec1-proof-observation-extension.md), imported from `d521e06d` and synchronized in this stack
>
> Scope: Phase 1 synchronous raw accounting, trusted deadline/internal stop, terminal arbitration and supervisor ownership

## Decision and sole authority

One request-owned `ExecutionBudget` is the only mutable raw/semantic accounting state and the only execution-winner cell.
It is created exactly once when an exact request row activates. The production `BytecodeVmBudget` is a private, non-Clone
adapter attached once to that budget; observers, artifacts, VM limits, fixtures, cancellation tokens and the supervisor
cannot create another adapter or mutate counters.

`RequestSupervisor` owns the keyed row, response action, terminal observation and cleanup transition. It does not own a
second terminal reason. Every completion, request cancel, session disconnect and budget failure first linearizes in the
same `ExecutionBudget`; the supervisor then consumes the returned immutable settlement. A cancellation token is wake-only
and is signalled only after that transition. The VM never reads it as decision authority.

```text
typed (RouterSessionEpoch, RequestId) activation
  -> one ExecutionBudget + one private BytecodeVmBudget attachment
  -> VM before_dispatch / semantic / poll calls
  -> one immutable ExecutionSettlement
  -> RequestSupervisor response action + terminal + DEC1-O cleanup permit
```

There is no raw quantum grant, precharge, refund, remainder, dispatch token, VM-side fuel integer or reset. The trusted
policy is finite `{ hard_raw_limit: 10_000_000, raw_poll_interval: 1024 }`; focused tests may use any `u64` limit, including
zero and `u64::MAX`. Production has no disabled/unlimited/optional-limit constructor, and request/artifact/image/manifest
data cannot set or enlarge either value. `VmLimits::raw_fuel_quantum` is removed; segment length remains only a scheduler
yield bound and grants no execution.

## Budget state and frozen outcome

The request crate owns these conceptual private types (names may vary only mechanically):

```rust
enum ExecutionWinner {
    Succeeded,
    Failed,
    Cancelled,                  // exact request.cancel winner
    DeadlineExceeded,
    InstructionLimitExceeded,
    InternalStop,               // session disconnect/runtime-owner stop
    AccountingFailure,
}

struct ExecutionBudgetState {
    raw_executed_count: u64,
    semantic_charge_count: u64,
    poll_count: u64,
    last_polled_raw_count: Option<u64>,
    vm_adapter_attached: bool,
    settlement: Option<Arc<ExecutionSettlement>>,
}

struct ExecutionSettlement {
    winner: ExecutionWinner,
    raw_executed_count: u64,
    semantic_charge_count: u64, // private; never a DEC1-O field
    hard_raw_limit: u64,
    poll_count: u64,
    started_at: Instant,
    finished_at: Instant,
}

enum SettlementDisposition {
    Won(Arc<ExecutionSettlement>),
    AlreadySettled(Arc<ExecutionSettlement>),
}
```

The settlement is built while holding the one budget-state mutex and is immutable thereafter. `AlreadySettled` can carry
each exact winner: succeeded, failed, cancelled, deadline exceeded, instruction-limit exceeded, internal stop or accounting
failure. It never recomputes counts/time and never replaces the winner. `Arc` clones expose frozen data only, not mutation.

The only non-VM transition methods are exact and accept no time supplied by a caller:

```rust
fn attach_vm(self: &Arc<Self>) -> Result<BytecodeVmBudget, VmAdapterAttachError>;
fn settle(&self, candidate: CompletionCandidate) -> SettlementDisposition; // Success | Failure
fn request_cancel(&self) -> SettlementDisposition;
fn request_internal_stop(&self) -> SettlementDisposition;
```

`attach_vm` is crate-private, locks the same state and rejects a closed or already-attached budget. `BytecodeVmBudget` has
no other constructor and contains only the exact budget reference; the canonical request driver consumes it rather than a
permit supplied to the VM. The unpublished alternate executors are removed as required by DEC1-O.

## Exact cross-crate VM port

The VM crate exposes this simple port and closed result:

```rust
pub enum VmBudgetTerminal {
    Succeeded,
    Failed,
    Cancelled,
    DeadlineExceeded,
    InstructionLimitExceeded,
    InternalStop,
    AccountingFailure,
}

pub enum VmBudgetClosed {
    DeadlineExceeded,             // selected by this call
    InstructionLimitExceeded,     // selected by this call
    AccountingFailure,            // selected by this call
    AlreadySettled(VmBudgetTerminal),
}

pub trait VmBudget {
    fn before_dispatch(&mut self) -> Result<(), VmBudgetClosed>;
    fn poll_interrupt(&mut self) -> Result<(), VmBudgetClosed>;
    fn charge_semantic(&mut self, charge: VmSemanticCharge<'_>) -> Result<(), VmBudgetClosed>;
}
```

This hard cut replaces `VmBudgetError`, `VmError::InvalidFuelGrant` and every old replenish/grant branch with the exact
`VmError::BudgetClosed(VmBudgetClosed)` carrier. The deleted names may not remain as aliases, deprecated wrappers or
conversions; `runtime/vm/src/control.rs` and `error.rs` migrate atomically.

`Succeeded`, `Failed`, `Cancelled` and `InternalStop` can reach the VM only as `AlreadySettled`; deadline, fuel and
accounting may be selected by the current call or returned later as `AlreadySettled`. Both forms map to the same dying-frame
terminal. There is no retry after any `VmBudgetClosed`.

`VmFiber` has one private wrapper:

```rust
fn dispatch_accounted(
    &mut self,
    heap: &mut dyn VmHeap,
    budget: &mut dyn VmBudget,
) -> Result<DispatchOutcome, VmError> {
    budget.before_dispatch().map_err(VmError::BudgetClosed)?;
    self.dispatch_one(heap)
}
```

`dispatch_one` remains private. No yield, callback, poll, semantic charge or other fallible bookkeeping occurs between the
successful call and exactly one invocation. Therefore `before_dispatch` atomically authorizes and charges the one attempted
dispatch, and an operand/type/invariant/opcode or language-semantic error returned by that invocation remains charged. A
`VmBudget` implementation cannot cause an extra VM instruction: it receives no fiber, heap, closure or dispatch capability.
The sole production implementation is the private attached adapter. A `RawDispatchReceipt` would add a forgeable
cross-crate token without strengthening this private call site and is rejected.

The loop first commits any pending function/source attribution using the existing cursor protocol, then calls
`dispatch_accounted`. A semantic-budget error before `before_dispatch` means no instruction was attempted and consumes no
raw unit. Once `before_dispatch` succeeds, later instruction failure never rolls raw accounting back. Successful
`charge_semantic` calls checked-increment only `semantic_charge_count`; they never consume raw capacity, mint dispatch
authority or appear as `chargedInstructionCount`.

## Raw boundary, polling and overflow

The adapter implements `before_dispatch` under the sole budget-state mutex:

1. return `AlreadySettled(exact winner)` if closed;
2. if `raw >= hard_raw_limit`, perform one forced fresh trusted poll, then select deadline if due or instruction limit;
3. otherwise, if the authoritative raw count is at a cadence boundary not already covered by an explicit poll, perform one
   trusted poll; the first dispatch boundary (`raw == 0`) is always covered;
4. checked-add exactly one raw unit and return `Ok(())`.

For limit `N`, calls 1 through N succeed and leave counts 1 through N. Completion at exactly N may win. Only call N+1
fails with fuel, leaves the count N and performs no dispatch. Limit zero fails on the first call. An instruction that fails
after its successful kth boundary leaves raw count k. No counter saturates or wraps.

The prior `raw >= hard_raw_limit` check makes raw overflow unreachable for every valid state. With limit `u64::MAX`, a state
at `MAX-1` advances exactly to `MAX`; the next call takes step 2, freezes instruction-limit exhaustion and never evaluates
`MAX + 1`. Therefore there is no raw-overflow terminal or test obligation. The checked add enforces the invariant; only
semantic and poll counters have reachable overflow cases.

`poll_interrupt` is used at every segment entry and verified `LoopCheck`. A trusted poll locks the same state, returns an
existing winner, takes an under-lock clock sample and computes a checked next `poll_count`. A due deadline wins; otherwise
poll overflow selects accounting failure, or the increment commits. A representable deadline-closing poll commits its
increment before freezing. A successful explicit poll records the current raw coordinate so `before_dispatch` does not
duplicate the same cadence boundary. Otherwise cadence is derived only from
`raw_executed_count % raw_poll_interval`; no VM-local remainder controls it. Thus at most 1024 successful dispatches occur
between cadence polls, while segment/loop polls may shorten the interval.

Semantic or poll overflow is fail-closed. Before selecting `AccountingFailure`, the operation samples the deadline under
the same lock so a due deadline has priority. Failed checked arithmetic changes no counter. Ordinary settlement and stop
deadline samples are arbitration samples, not VM polls, and do not inflate `poll_count`.

## Trusted clock and deadline priority

`ExecutionBudget::new(policy, admitted_deadline, clock)` receives and retains an `Arc<dyn TrustedMonotonicClock>`; production
uses the host-owned system implementation and focused tests inject a controllable monotonic clock. `started_at` is sampled
at construction. No budget transition accepts `Instant`, elapsed duration or a caller `now`.

For every open-state `settle`, `request_cancel` and `request_internal_stop`, the implementation first acquires the budget
mutex and only then samples its retained clock. If `now >= admitted_deadline`, it freezes `DeadlineExceeded`; otherwise it
freezes the requested candidate. Poll/fuel/accounting terminal selection follows the same due-deadline-first rule. If a
winner already exists, the method returns it without another sample or mutation.

Ingress normalizes the typed wire timeout/expiry once using trusted host wall/monotonic clock sources and carries one typed
absolute `AdmittedRequestDeadline` into activation; `ExecutionBudget` never reparses `RequestEnvelope.extra`. Invalid or
unrepresentable input fails admission. A deadline already due is rejected in `spawn_bytecode_request` before observer
construction, reservation, deployment load or activation, with the stable correlated deadline response, zero observations
and no row/budget. A deadline becoming due later is resolved by the budget.

This gives exact priority rather than thread scheduling priority: a D-1 raw charge followed by a stop/completion that acquires
the lock at D+1 freezes deadline; success that freezes at D-1 remains success even if a deadline waiter runs later.

## Typed request/session ownership

The supervisor table is keyed by typed identity, never by request-id text alone:

```rust
struct RouterSessionEpoch(/* opaque connection epoch */);
struct RequestId(/* validated wire request id */);
struct RequestExecutionKey {
    router_session: RouterSessionEpoch,
    request_id: RequestId,
}

enum ActivationOutcome {
    Activated(ActivatedRequest),
    RevokedByCancel,
    RevokedBySessionStop,
    Invalid,
}
```

One epoch is minted for each connected Router WebSocket. `spawn_bytecode_request`, observer correlation, reservation,
activation, completion, cancel and cleanup all retain the exact key. Consuming
`RequestReservation::activate(self, exact_key, ...)` returns the exact `ActivationOutcome`, never `Option`; it creates the
budget/active carrier under the row lock only after matching `Reserved`. Assembly entry functions retain the typed key.

The `request.cancel` handler combines the current connection epoch with the validated frame `RequestId`. A missing exact row
is rejected; session B can never cancel session A's equal id. Under the row lock, an exact `Reserved` row is revoked so its
holder cannot activate; an exact `Active` row invokes `request_cancel()` (which locks the budget and samples its clock) and
records the frozen disposition. The row lock is released before signalling the wake token or emitting telemetry.
`Completing` and `Cleanup` are already claimed and never reopen. Thus cancel racing activation either returns
`RevokedByCancel` or returns `Activated` whose exact budget selects cancel/deadline.

`ConnectedRouterSessionGuard::close` calls `RequestSupervisor::stop_session(epoch)` before releasing session transports.
Under the row lock it revokes every matching `Reserved` row and, for every matching `Active` row, independently calls
`request_internal_stop()`. Each active budget therefore acquires its own lock and samples its own retained clock after that
lock; disconnect never passes one shared caller timestamp. Wake handles are collected under the row lock and signalled only
after it is released. Other epochs and already-completing/cleanup rows are untouched. A reserve/activate race therefore
returns `RevokedBySessionStop` or returns `Activated` whose exact budget selects internal-stop/deadline.

Both revoked outcomes map once to `StopWithoutResponse`. They mint no `ExecutionBudget` or DEC1-O inventory, perform no
settlement/terminal observation/cleanup, and consume the exact revoked reservation tombstone once. `Invalid` means the
consumed token/key/row identity did not match and maps to admission error `bytecode request reservation activation failed`,
also without budget/inventory/terminal/cleanup. Cancel-versus-session-stop preserves the first revocation kind.

Lock order is `supervisor row -> ExecutionBudget`; VM paths lock only `ExecutionBudget`; observation and wake callbacks run
under neither. No code may acquire a row lock while holding a budget lock.

## Completion and terminal races

The worker submits only `CompletionCandidate::Success(response)` or `Failure(error)`. While holding the exact active row,
the supervisor calls `budget.settle(candidate.kind())`, stores the returned frozen settlement in `Completing`, and derives
one action solely from `settlement.winner`:

| Frozen winner | Sole supervisor action | `RequestTerminalClaimed` |
| --- | --- | --- |
| `Succeeded` | `RespondSuccess(candidate response)` | `Succeeded` |
| `Failed` | `RespondFailure(candidate typed failure)` | `Failed` |
| `DeadlineExceeded` | stable `TimeoutError`: message `execution deadline exceeded`, reason `deadlineExceeded`, frozen raw count/limit/elapsed | `Failed` |
| `InstructionLimitExceeded` | canonical `std.error.InstructionLimitExceededError { instructionCount, limit }` | `Failed` |
| `Cancelled` | `StopWithoutResponse` | `Cancelled` |
| `InternalStop` | `StopWithoutResponse` | `Cancelled` |
| `AccountingFailure` | sanitized stable `InternalError` while response is owned | `Failed` |

A candidate payload is usable only when its matching success/failure winner is frozen; it cannot override a prior terminal.
Deadline/fuel/cancel/internal stop/accounting kill the frame and are not catchable. The action owns the unique cleanup permit;
no response or terminal observation occurs before the row claim.

| Race | Required linearized result |
| --- | --- |
| success/failure vs cancel/internal stop | first budget transition wins; supervisor consumes it |
| deadline vs any open transition | that transition's under-lock clock sample chooses due deadline first |
| fuel/accounting vs deadline | fresh under-lock sample chooses due deadline, otherwise the budget failure |
| exact-limit completion vs N+1 call | completion at N may win; N+1 may instead freeze fuel; first lock wins |
| raw charge vs stop | stop first rejects dispatch; charge first accounts exactly the one attempted dispatch |
| duplicate completion/late cancel | exact row is no longer Active; no second action/event/permit |
| observer failure | settlement, action and cleanup ownership are unchanged |

## DEC1-O observation and cleanup join

Corrected DEC1-O in this decision stack is the sole authority for observation fields and request-owner inventory. The supervisor
projects exactly one `VmBudgetAccounted` from the frozen settlement immediately before the separate terminal event:

```text
{ rawExecutedCount, chargedInstructionCount, hardLimit, pollCount }
```

`chargedInstructionCount` is derived equal to `rawExecutedCount`. There is no semantic count, terminal reason, outcome or
verdict, and no unused raw tail. The observer neither reads mutable stats nor participates in settlement.

Budget settlement does not create a cleanup ledger. DEC1-O's actual `RequestExecutionOwnerInventory` remains one locked
current-plus-ever-created authority whose uncloneable leases live in the real `PendingCell`, `ResourceEntry` and
`BlockedUnit`. Pending/resource/child counts and their three `everCreated` bits are frozen once and carried byte-for-byte in
the unique cleanup permit. A stale completion handle without the carrier's owner lease is not a live owner.

The Phase 1 canonical driver physically omits the pending registry/wake queue, resource table and child/stream ports. Its
`NotStarted` finish state is valid only after that structural absence is proved and the actual DEC1-O inventory is sealed
before execution; its zero/false fields come from that sealed inventory, never `Default`, a constant or observer synthesis.
A `Started` finish state carries the actual inventory snapshot frozen at the driver terminal transition. Start failure,
success, VM failure, budget terminal and internal stop must all deliver one of those sealed/frozen actual snapshots before
supervisor completion. Later drops cannot resample or normalize it.

## Atomic migration and exact write set

After reviewed K1 joins, L4/L5 lands as one ordered hard cut: typed clock/deadline/key and budget state; VM port/private
dispatch wrapper; private request adapter and all trait consumers; supervisor/session terminal arbitration; corrected
DEC1-O inventory/result/cleanup carriers; then Proof migration. No precharge adapter, request-id-only supervisor, token winner,
legacy alternate executor or synthetic cleanup path is joinable.

| Lane | Exact files |
| --- | --- |
| VM port/call site | `runtime/vm/src/{lib,budget,control,error,fiber,limits,statement}.rs`; `runtime/vm/src/fiber/{tests,projection_tests}.rs`; `runtime/vm/src/statement/tests.rs`; `runtime/vm/tests/vertical.rs` |
| Request budget/adapter | `runtime/request/src/{lib,execution_budget,execution_control,bytecode_ingress,continuation_handoff,runner,error}.rs`; `runtime/request/src/{execution_budget/tests,execution_control/tests,error/tests}.rs`; `runtime/request/tests/bytecode_request.rs` |
| Scheduler consumers | `runtime/scheduler/src/{lib,bytecode}.rs`; `runtime/scheduler/tests/bytecode_scheduler.rs` |
| Keyed host lifecycle | `runtime/host/src/host/{runtime_host,request_supervisor,router_session,request_entry}.rs`; `runtime/host/src/host/request_entry/{assembly,assembly_wire,resumable,websocket_jsonrpc,bytecode_http_tests,phase_0_negative_tests,tests}.rs`; `runtime/host/src/host/router_session/tests.rs`; `runtime/host/src/host/router_session/tests/{control_response_lifecycle,h_task_parent_cut}.rs` |
| DEC1-O actual inventory | `runtime/scheduler/src/{owner_inventory,lib,pending,trampoline,bytecode,stream_driver}.rs`; `runtime/scheduler/tests/bytecode_scheduler.rs`; `runtime/request/src/{vm_heap,bytecode_ingress,continuation_handoff,lib}.rs`; `runtime/request/src/vm_heap/tests.rs`; `runtime/request/tests/bytecode_request.rs`; `runtime/host/src/host/request_entry/{resumable,assembly,websocket_jsonrpc}.rs`; `runtime/host/src/host/request_supervisor.rs` |
| DEC1-O model/VM/proof | `runtime/model/src/bytecode_execution_observation.rs`; `runtime/vm/src/fiber.rs`; `runtime/host/src/host/request_entry/{phase_0_vcp_tests,phase_1_runtime_proof_support,phase_1_runtime_proof_tests}.rs`; `runtime/host/src/host/request_entry/phase_1_runtime_proof_support/observations.rs` |

All `VmBudget` implementations and `VmLimits::new` callers in VM/request/scheduler focused tests migrate in the same commit.
All host completion call sites supply the DEC1-O sealed/frozen inventory carrier. A newly discovered production caller returns
to MAP1 for write-set addition; it is not kept alive with a compatibility constructor. No compiler, artifact/schema,
linker, Cargo manifest, Gate selector or registry edit is authorized by this decision.

## Required focused proof

Deterministic fake-clock/barrier tests must prove:

1. exactly one private adapter; no quantum/tail/token; raw 1 after one dispatch; instruction-error charge; N/N+1 and zero;
2. limit `u64::MAX` with `MAX-1 -> MAX -> N+1 fuel`, semantic/poll overflow, semantic independence and counter-derived cadence;
3. first/segment/loop polls, no duplicate cadence poll at one raw coordinate, and no retry after every closed result;
4. each `AlreadySettled` winner maps exactly, including success, failure, cancel, deadline, fuel and internal stop;
5. clock samples occur after budget-lock acquisition for settle/cancel/stop and separately for every disconnected row;
6. D-1/D+1 deadline barriers and every completion/fuel/cancel/internal-stop first-winner race;
7. expired/invalid ingress creates no observer, reservation or budget and emits zero observations;
8. activate-before/after-cancel, activate-before/after-session-stop, cancel-vs-stop first revoke, invalid key/row identity
   and revoked-tombstone consumption; both revoke outcomes act once without settlement/terminal/cleanup;
9. equal request ids in sessions A/B coexist; B cancel cannot affect A; disconnect revokes reserved and stops active A only;
10. one supervisor action/terminal/cleanup permit, distinct deadline/fuel/cancel/internal-stop projection, observer isolation;
11. exact four-field DEC1-O budget serialization and actual sealed `NotStarted`/frozen `Started` inventory propagation.

Reverse search must find no whole-quantum charge/refund, raw remainder or receipt, VM fuel counter, semantic-to-raw charge,
caller-supplied transition time, token/cancel-flag winner, request-id-only row, discarded assembly session epoch, completion
override, mutable post-settlement stats, default/constant cleanup zero, alternate owner ledger or unleased actual carrier.

## Rejected alternatives and closure

Whole-quantum grants require refund and misreport unused tails. A move-only receipt cannot prove more than the VM-private
adjacent call and creates a cross-crate forging surface. Semantic charge is attribution, not raw work. Caller `now`, a shared
disconnect timestamp or completion-time cancel inspection creates competing time/terminal authority. Constant cleanup zero
proves an allowlist rather than actual ownership. Outcome fields in `VmBudgetAccounted` duplicate the terminal event.

No shared semantic choice remains. Independent review must cover the per-dispatch boundary, exact closed-result mapping,
deadline sampling/priority, keyed disconnect/cancel races, supervisor frozen-winner consumption, DEC1-O four-field projection,
actual inventory `NotStarted`/`Started` carriers and the complete migration/write set before L4/L5 starts.
