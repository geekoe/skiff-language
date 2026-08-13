PASS

# REV1-L5: independent owner-inventory correction review

> Status: PASS for the four-commit correction chain `296462db..6d0d215b`, with one
> blocker-level pre-existing finding routed to the integrator (see Findings).
> Reviewer: independent read-only L5 review role; not a production writer. No
> code was written, no test was run, nothing was committed.

## 1. Reviewed

- Range `git diff 296462db^..6d0d215b`, i.e. the four commits:
  - `296462db` scheduler: replace split owner inventory with opaque execution context
  - `add921ac` request: own sole drive facade and delete dead async paths
  - `86164aef` test(request): match canonical native-wrapper containment
  - `6d0d215b` host: carry frozen owner inventory through supervisor cleanup
- Current `HEAD` is `e2e19233`, docs-only above `2be7d126` (R0 baseline +
  MAP1/acceptance-baseline text). Production tree reviewed is `6d0d215b`;
  `git diff 6d0d215b..HEAD` touches only
  `doc/implementation/bytecode-vm-convergence/tasks/{phase-1-acceptance-baseline.md,phase-1-execution-map.md}`.
- Documents used as contract: DEC1-O (`dec1-proof-observation-extension.md`,
  accepted as `8fb50a84`), DEC1-B (`dec1-budget-and-stop-ownership.md`, accepted
  as `824c4616`), MAP1 Revision 10 (`17.` blocker list) and Revision 11
  (`18.` correction description), and
  `phases/phase-1-trusted-synchronous-core.md` §10 "L5 — Unary request boundary
  and terminal".
- Prior owner's evidence accepted as stated, not re-run (cargo/test/build/node
  are forbidden for this role): three-package `cargo test` exit 0, log
  `/tmp/skiff-p1-l5-correction-full.log`, SHA-256
  `851d52fa168f6b21f58bb31d90256e17d798c1b4af09fc393f844355c392c476`.
- Review method: static read of the diff and current tree only
  (`git log/show/diff/status`, `rg`, `sed`, `cat`, `wc`). All claims below carry
  file:line evidence against the `6d0d215b` tree.

## 2. Verdict

The three mandatory Revision 10 corrections are structurally complete. The
scheduler's only public inventory authority is the non-cloneable, opaque,
owner-bound `RequestExecutionContext`; installation is lock-ordered
prepare-alloc → inventory → container → unarmed placeholder → infallible
`commit()`; owner-creation failures are domain-tagged and projected to one
sanitized request error; and the frozen snapshot travels byte-for-byte through
`CompletingRequest -> CleanupPermit -> CleanupGuard -> RequestCleanupComplete`.

One finding is recorded as blocker-level for the *next* owner but is pre-existing
to this chain: the accepted Phase 0 host-level blocked-callback/finalizing-race
test was deleted in L4 (`2e24763b`), before this range, and this chain does not
restore it. The guarded-identity behavior it proved is still present; the
regression test is not. See Findings.

## 3. Theme A — opaque execution context

**PASS.**

- The only public entry into one owner inventory is
  `RequestExecutionContext<U>` (`runtime/scheduler/src/owner_inventory.rs:437`).
  It has no `Clone`/`Copy`/`Default` derive and its fields are private.
- It is consumed in exactly two ways, both taking `self` by value:
  `into_not_started(self)` freezes `NotStarted(actual snapshot)`
  (`owner_inventory.rs:475`), and `drive(self, heap, budget)` constructs the
  scheduler, calls `run` exactly once, then freezes `Started(actual snapshot)`
  unconditionally on every `run` outcome — `Complete`, `Parked`, and every
  `Err` — before returning `(result, snapshot)` (`owner_inventory.rs:485-515`,
  `runtime/scheduler/src/bytecode.rs:367`).
- All previously public split parts are now scheduler-private:
  `PendingOwnerRegistration` / `ResourceOwnerRegistration` /
  `ChildOwnerRegistration` (`owner_inventory.rs:240,243,246`), the three
  creation guards and leases, `RequestExecutionOwnerRegistrations`,
  `RequestExecutionOwnerInventory` (`:377`), `open()` (`:383`),
  `into_parts()` (`:395`), and the freeze permit + `freeze()` (`:405-421`).
  The generic `install` closure is deleted; `prepare()`/`commit()` are
  `pub(crate)` (`:318,324`).
- The public scheduler entry points that remain (`PendingRegistry::new`,
  `FlatTrampoline::new`, `BytecodeScheduler::run`,
  `resume_from_pending_wake`, `resume_from_suspended`) are sealed behind
  `pub(crate)` registration argument types or consume already-created carriers
  (`pending.rs:574`, `trampoline.rs:64`, `bytecode.rs:329-343,474,492`). They
  cannot mint, forge, or mix inventory authority.
- Counterexample search: a caller cannot install a counted fake carrier
  (no public `prepare`/`install`/lease mint), cannot mix request A registrations
  with request B's freeze permit (both are minted by the same private `open()`
  and only leave it as one `RequestExecutionContext`), and cannot share one
  owner state across two requests (`create()` calls `open()` fresh,
  `owner_inventory.rs:449-459`).

## 4. Theme B — unarmed placeholder + infallible commit

**PASS.**

- `FlatTrampoline::enter_child` (`trampoline.rs:84-106`): `try_reserve`
  (allocation) happens before the inventory lock; `prepare()` holds the
  inventory lock; then only an unarmed placeholder `BlockedUnit { owner_lease:
  None }` is pushed with `std::mem::replace`; `guard.commit()` is infallible
  (overflow pre-checked at prepare, `owner_inventory.rs:172-187,209-224`); the
  lease is then attached. No caller code runs between the inventory lock
  acquisition and release, and `&mut self` excludes reentrancy.
- `PendingRegistry::begin` → `install_with_guard`
  (`pending.rs:590,623-660`): inventory lock (via guard) is held before the
  container `cells` lock; the unarmed `PendingCell` is inserted, `commit()`
  mints the lease, and `cell.arm(lease)` (`pending.rs:361-374`) attaches it
  while the table lock is still held, so no other operation can observe an
  unarmed cell.
- No panic/reentrancy is possible between the increment and the unlock:
  `commit` performs only `current += 1; ever_created = true` under the already
  held guard (overflow unreachable because `prepare` rejected
  `current == u64::MAX`) and returns a lease whose `Drop` locks the inventory
  only after `commit` released the guard. The former `install`-closure
  deadlock (increment under lock, then caller code panics, then lease Drop
  re-locks the same mutex) is structurally impossible.
- Resource table: deleted from `runtime/request/src/vm_heap.rs`
  (`ResourceRegistry`/`ResourceTable`/`set_resource_table` removed;
  `ResourceRef` now fails closed in `ValidateLive`/`TransferOwner`/
  `ReleaseResource`, `vm_heap.rs:512-522,563-568,629-636`), with focused
  fail-closed tests retained (`runtime/request/src/vm_heap/tests.rs:199-263`).

## 5. Theme C — domain-tagged owner-creation error

**PASS.**

- One shared `OwnerCreationError { domain, kind }` carries
  `OwnerDomain::{Pending,Resource,Child}` ×
  `OwnerCreationErrorKind::{InventoryFrozen,CountOverflow}`
  (`owner_inventory.rs:20-23,38-41,58-78`; fields private with `domain()`/
  `kind()` accessors — the struct-literal form in the review prompt is
  satisfied semantically). All three domains use it via the same
  `registration_guard!` macro (`owner_inventory.rs:311-345`).
- Container errors remain distinct:
  `BeginPendingError::TicketCollision` (`pending.rs:590-641`, plus
  `TicketSpaceExhausted`) and `EnterChildError::CapacityExceeded`
  (`trampoline.rs:8-24`) are separate variants outside `OwnerCreationError`;
  `BytecodeSchedulerError::ChildCapacityExceeded` and
  `::ChildOwnerCreation(OwnerCreationError)` are likewise distinct
  (`bytecode.rs:20-30`).
- Request-side projection: `BytecodeSchedulerError::ChildOwnerCreation(_)`
  maps to the single sanitized
  `RequestError::Decode("bytecode scheduler owner creation failed")` with the
  inner error discarded (`runtime/request/src/bytecode_ingress.rs:397-421`).
  `RequestError::Decode` serializes with wire `code: "InternalError"` and that
  exact message, leaking no internal domain/kind details
  (`runtime/request/src/error.rs:95-101`). This is the intended
  "sanitized InternalError" projection.

## 6. Theme D — snapshot through the supervisor cleanup chain

**PASS.**

- The request crate's only public composition is
  `drive_runtime_bytecode_request` (`bytecode_ingress.rs:89-175`). It creates
  the context (`BytecodeSchedulerPorts::default()`, both ports `None`), starts
  the fiber exactly once, and returns exactly one of
  `DrivenBytecodeRequestOwnerInventory::{NotStarted(snapshot), Started(snapshot)}`
  plus the opaque `BytecodeRequestRetention`
  (`bytecode_ingress.rs:52-81`). Start failure freezes `NotStarted` before any
  drive (`:129-141`); a completed drive freezes `Started` on success, park, and
  error alike (`:166-175`). No `_with_ports` or legacy
  `start_runtime_bytecode_request` remains (`runtime/request/src/lib.rs:8-11`).
- Host no longer mints/splits/freezes inventory: the deleted
  `drive_bytecode_request` composition, orphaned `bytecode_http_executor.rs`,
  `resumable.rs`, and the old request-crate
  `continuation_handoff.rs`/`http_executor.rs`/`response_stream_writer.rs`/
  `response_writer.rs` are removed, not re-exported
  (`runtime/host/src/host/mod.rs`, `request_entry.rs`,
  `runtime/request/src/lib.rs`).
- Every admitted completion consumes the frozen snapshot:
  `complete_success/failure/fixed_service_failure/cancelled` take
  `RequestExecutionOwnerInventorySnapshot` (`request_supervisor.rs:289-348`),
  store it in `CompletingRequest` (`:131-136,467-481`), move it into
  `CleanupPermit` (`:203-211,781-790`), then into `CleanupGuard` (`:213-220,
  650-690`), and `observe_cleanup` emits
  `RequestCleanupComplete { owner_inventory }` (`:694-703`). The snapshot type
  is `Copy`; it is never resampled or normalized. `begin_cleanup` additionally
  verifies `completing.owner_inventory == owner_inventory` plus
  `Arc::ptr_eq` on row identity and settlement (`:668-674`).
- Host call sites (`assembly.rs:95-127,170-227,282-314,359-380`;
  `websocket_jsonrpc.rs:89-122`) collapse `NotStarted|Started` once via
  `into_snapshot()` and pass the same value into every completion branch,
  dropping the retention carrier only after completion and cleanup ordering
  (`drop(retention)` before `observe_bytecode_request_cleanup`).
- Pre-activation `RevokedByCancel`/`RevokedBySessionStop`/`Invalid` return
  before `drive_runtime_bytecode_request`, so no inventory, budget, terminal or
  cleanup exists (`assembly.rs:70-85`, `websocket_jsonrpc.rs:71-83`);
  supervisor test asserts the sink stays empty
  (`request_supervisor.rs:1008-1034`).

## 7. DEC1-O independent review questions 6–9

### Q6 — every actual owner requires its uncloneable lease at its sole constructor

**PASS.**

- Pending cell: sole production constructor is
  `begin`/`begin_with_ticket` → `install_with_guard`, which arms the
  just-inserted cell with the minted `PendingOwnerLease` before the table lock
  is released (`pending.rs:590,623-660,361-374`). `PendingCell::new` is private
  and its placeholder is never observable. Cloned `CompletionHandle`s share the
  one cell and do not duplicate the lease.
- Blocked child: sole constructor is `enter_child` (`trampoline.rs:84-106`);
  the `BlockedUnit` placeholder is armed immediately under `&mut self`.
- Resource entry: no longer exists (table deleted, Theme B). The resource
  domain therefore has no production constructor at all — strictly stronger
  than required, and consistent with DEC1-O's canonical physical-absence
  containment.
- No untracked/default path: `OwnerLease` is minted only by `commit()`, the
  typed leases are `pub(crate)`, and `PendingOwner::into_parts`'s
  `Option<PendingOwnerLease>` cannot be fabricated externally
  (`pending.rs:92-94`).

### Q7 — lease continuity and create/release/freeze races

**PASS.**

- The same `Option<PendingOwnerLease>` value moves through
  `Open -> Waiting`, `Open -> Settled`, `Settled -> PendingWake`, and
  `Waiting -> PendingWake` without release/reacquire
  (`pending.rs:285-310,377-441`). `BytecodeScheduler::resume_from_pending_wake`
  retains the leased wake through root restoration and drops the owner only
  after resume succeeds or fails (`bytecode.rs:474-490`).
- Race proof in tests: create-vs-freeze barrier
  (`owner_inventory.rs:558-586`), release-then-freeze and freeze-then-drop
  orderings (`owner_inventory.rs:517-533`), freeze while the leased wake is
  live (`pending.rs:935-960`), pending installation holding inventory-before-
  registry lock order (`pending.rs:993-1037`), and frozen rejection of a
  started child without installing it
  (`runtime/scheduler/tests/bytecode_scheduler.rs:303-375`). An installed,
  even ephemeral, owner therefore cannot yield `current == 0` with
  `ever_created == false`: both counters are written under one inventory lock
  before the lease is minted, and freeze reads both under the same lock.

### Q8 — one immutable snapshot frozen at the driver terminal, propagated byte-for-byte

**PASS.**

- `freeze()` asserts `Open`, sets `Frozen`, and returns the fixed-size
  `RequestExecutionOwnerInventorySnapshot` (`owner_inventory.rs:410-421`);
  the unique permit is consumed. `drive` freezes after `run` returns for every
  outcome (`owner_inventory.rs:485-515`); start failure freezes before any
  drive (`bytecode_ingress.rs:129-141`).
- The `Copy` snapshot is carried by value through `CompletionWinner ->
  CompletingRequest -> CleanupPermit -> CleanupGuard -> RequestCleanupComplete`
  (`request_supervisor.rs:467-481,650-703,781-790`) with the equality
  cross-check at `begin_cleanup` (`:668-674`).
- Host regression proves a non-zero snapshot (`pending: current=1,
  ever_created=true`) is emitted byte-for-byte on `RequestCleanupComplete`
  (`request_supervisor.rs:1081-1137`), and the Phase 0 VCP test asserts the six
  zero/false facts at ordinal 4
  (`runtime/host/src/host/request_entry/phase_0_vcp_tests.rs:191-210`).

### Q9 — Active → Completing → Cleanup identity and blocked/reentrant behavior

**PASS (behavior), with a test-retention finding (see Findings).**

- `claim_completion` requires the row to be `Active` with `Arc::ptr_eq` row
  identity before inserting `Completing` (`request_supervisor.rs:453-487`);
  `begin_cleanup` requires `Completing` with matching row identity,
  settlement, and snapshot before inserting `Cleanup` under a fresh
  `guard_identity` (`:650-690`); the row is removed only in
  `CleanupGuard::finish`, which runs after the sink callback returns
  (`:704-711`). A blocked terminal or cleanup sink therefore keeps the row
  claimed, and late reserve/cancel/completion cannot reopen it.
- Reentrancy/panic isolation tests remain in the model:
  `concurrent_barrier_delivery_is_strictly_ordinal`,
  `reentrant_observation_is_queued_and_panic_does_not_stop_drain`,
  `reentrant_queue_is_bounded_and_overflow_is_dropped`
  (`runtime/model/src/bytecode_execution_observation.rs:308-342`).
- Gap: the host-level blocked-callback/finalizing-race test
  `request_id_stays_guarded_through_terminal_and_cleanup_observers` (with
  `LifecycleBlockingSink`) was deleted in L4 `2e24763b`, before this range, and
  is not restored here. DEC1-O's "Tests and Proof migration" explicitly
  requires retaining it. See Findings.

## 8. Revision 10 blocker checklist

Each blocker from MAP1 Revision 10 (authoritative handoff) is resolved in the
corrected tree:

1. **Public split inventory surface** — resolved: all registrations, guards,
   leases, `open`, `into_parts`, and the generic `install` closure are
   `pub(crate)` or deleted (`owner_inventory.rs:240-246,311-345,377-421`).
2. **Counted fake carrier install** — resolved: no public prepare/commit/
   install exists; the only public entry mints and consumes everything
   internally (Theme A).
3. **Mixing request A registrations with request B freeze permit** — resolved:
   both are minted together by one private `open()` and are inseparable in
   `RequestExecutionContext` (Theme A).
4. **Panic-under-lock deadlock** — resolved: the install closure is gone;
   `commit()` runs no caller code and is infallible (Theme B).
5. **Two requests sharing one owner state** — resolved: `create()` opens a
   fresh `InventoryShared` per context; no shared/parametrized constructor.
6. **Canonical Phase 1 ports/tables physically absent (`None`)** — resolved:
   the sole drive facade constructs `BytecodeSchedulerPorts::default()`
   (`bytecode_ingress.rs:107`); no resource table is installed in
   `RequestVmHeap`; unexpected park fails with `Unsupported` on the
   synchronous lane (`bytecode_ingress.rs:160-163`).
7. **Structural proof that `Option` ports cannot be populated / constructors
   unreachable without a guard** — resolved: `absent_ports_fail_closed`
   (`bytecode_scheduler.rs:527-564`), `frozen_inventory_rejects_a_started_child...`
   (`:303-375`), resource fail-closed heap tests (`vm_heap/tests.rs:199-263`),
   and constructor sealing by `pub(crate)` registration argument types
   (`pending.rs:574`, `trampoline.rs:64`, `stream_driver.rs:166,354`).
8. **Pre-activation revoke outcomes create nothing and never double-settle** —
   resolved: early return on `RevokedByCancel`/`RevokedBySessionStop` and
   admission-error-only `Invalid` before any drive/completion
   (`assembly.rs:70-85`, `websocket_jsonrpc.rs:71-83`,
   `request_supervisor.rs:1008-1034`).
9. **Request crate sole public composition; host must not mint/freeze; dead
   paths deleted** — resolved: `drive_runtime_bytecode_request` only; host has
   no `RequestExecutionOwnerInventory::open`/`freeze()` call;
   `drive_bytecode_request` and `bytecode_http_executor` deleted;
   adapter/Pending/stream/resume implementations and the resource table deleted
   rather than re-exported (Themes C, D).
10. **Snapshot through `CompletingRequest -> CleanupPermit -> CleanupGuard`
    and `RequestCleanupComplete` gains fields** — resolved in `6d0d215b`
    (Theme D).

## 9. Deviation adjudication — `RequestCleanupComplete` payload shape

**Adjudication: Option N (keep the nested shape).**

Rationale. The nested shape violates no hard constraint of DEC1-O's typed event
contract: the six facts exist exactly once with a 1:1 semantic mapping, every
leaf is a fixed-width scalar (`u64` / `bool`), the `kind`/`payload` envelope and
camelCase serialization are retained, there is no `Default`/synthesis, and
cardinality/order semantics are unchanged. DEC1-O's "Remaining implementation
choices" explicitly leaves the fixed-size inventory state and snapshot
names/layout to L5 ("L5 may choose private names/layout for the fixed-size
inventory state and snapshot"), which covers the group/name difference. No
downstream typed consumer exists yet that pins the flat names (O1 projection,
T-R/V1 matching, and Gate schema all land after this review), so Option N
breaks nothing; Option F would require evidence of an actual hard-constraint
violation, which does not exist, and would churn the already-corrected carrier
for no semantic gain.

Final serde shape (R2 production, R3 T-R typed matching, R4 Gate JS schema must
all use these exact names):

```json
{
  "kind": "RequestCleanupComplete",
  "payload": {
    "ownerInventory": {
      "pending":  { "current": 0, "everCreated": false },
      "resource": { "current": 0, "everCreated": false },
      "child":    { "current": 0, "everCreated": false }
    }
  }
}
```

Field paths (relative to `payload`, Rust source
`runtime/model/src/bytecode_execution_observation.rs:233-257`):

| DEC1-O flat fact | Final field path | Type |
| --- | --- | --- |
| `pendingOwnerCount` | `ownerInventory.pending.current` | `u64` |
| `pendingOwnerEverCreated` | `ownerInventory.pending.everCreated` | `bool` |
| `resourceOwnerCount` | `ownerInventory.resource.current` | `u64` |
| `resourceOwnerEverCreated` | `ownerInventory.resource.everCreated` | `bool` |
| `childOwnerCount` | `ownerInventory.child.current` | `u64` |
| `childOwnerEverCreated` | `ownerInventory.child.everCreated` | `bool` |

Delta required against DEC1-O: amend the "Typed event contract" struct sketch
to replace the flat six-field `RequestCleanupComplete` with the nested
`{ owner_inventory: { pending|resource|child: { current, ever_created } } }`
shape, record the 1:1 six-fact mapping above as the adjudicated names, and note
the change is authorized by the "Remaining implementation choices" snapshot
layout clause. `RequestCleanupComplete` must keep losing `Default`
(`bytecode_execution_observation.rs:255-257` derives no `Default`).

## 10. Findings

- **Blocker-level (pre-existing, outside reviewed range; route to integrator /
  next owner before O1/merged Gate):** the accepted Phase 0
  blocked-callback/finalizing-race regression
  `request_id_stays_guarded_through_terminal_and_cleanup_observers`
  (+ `LifecycleBlockingSink`) was deleted by L4 `2e24763b` and is not restored
  in the current tree (`rg LifecycleBlockingSink` → none). DEC1-O "Tests and
  Proof migration" requires "retain the accepted blocked-callback/finalizing-
  race tests". The guarded behavior itself remains intact (row stays
  `Completing` during a blocked terminal sink, `Cleanup` under
  `guard_identity` during a blocked cleanup sink — `request_supervisor.rs:
  453-487,650-711`), and reentrancy/panic tests remain in the model, but the
  host-level race regression must be reintroduced (typed-key equivalent) before
  acceptance. This does not fail the reviewed four-commit chain because the
  deletion predates it and the chain neither removed nor weakened the
  machinery.

## 11. Residual notes

- `OwnerCreationError`'s fields are private with `domain()`/`kind()` accessors
  rather than public struct fields; this is name/layout freedom and preserves
  the required domain × kind product.
- The sanitized projection uses `RequestError::Decode(...)` whose wire code is
  `InternalError` (`error.rs:95-101`); there is no dedicated
  `RequestError::InternalError` enum variant. The observable sanitized error is
  the required one.
- `BytecodeSchedulerPorts` fields remain public and the scheduler retains
  public `run`/`resume_from_pending_wake`/`resume_from_suspended`; these are
  phase-2 scheduler surfaces that consume existing carriers and cannot mint
  inventory (registrations are `pub(crate)`). The canonical Phase 1 request
  path never supplies populated ports.
- The resource domain keeps `ResourceOwnerRegistration`/guard/lease in
  `owner_inventory.rs` but has no production constructor after the
  `ResourceTable` deletion; the snapshot's resource domain is therefore always
  `current == 0, ever_created == false` on the synchronous lane, by structural
  absence rather than synthesis.
- The model crate has no explicit serde-string regression for the new snapshot
  field names yet; the derives are unambiguous. Recommend O1/R3 add the exact
  serialization test per the adjudicated names in §9.
- The noted pre-existing workspace-wide `cargo fmt --check` drift (652 files)
  and the untouched `compiler/emission/.../admission.rs:60` clippy lint
  (MAP1 Revision 11) are out of the write set and not introduced here.
