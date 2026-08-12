# DEC0-S: Phase 0 production VCP seam and observation contract

> Status: decided
>
> Input: `dc7080fe4c18358e3cc3cd2d2cf4a56ca94b1552`
>
> Scope: VCP placement and the minimum read-only observation contract only

## Decision

The canonical Phase 0 VCP is a `runtime/host` crate-internal request-entry test.
It enters through the existing `RuntimeHost::spawn_bytecode_request` method and
does not call `BytecodeDeploymentRegistry`, loader, linker, verifier, image,
route, request, scheduler, or VM constructors directly.  The test belongs in
`runtime/host/src/host/request_entry/phase_0_vcp_tests.rs`, registered only as a
normal `#[cfg(test)]` child module of `request_entry`.  No production method is
made public and no test-only execution facade is added.

This is the smallest placement that can cross the production ownership chain.
`spawn_bytecode_request` already owns the wire request correlation and dispatch
at `runtime/host/src/host/request_entry/assembly_wire.rs:67-182`; its route
resolution calls the crate-private registry at `assembly_wire.rs:184-199`.
That registry alone performs exact store hydration, link, verify, image-cache
load and route construction (`runtime/host/src/loader/bytecode_admission.rs:57-139`).
Moving the proof down to `runtime/request` cannot reach those owners because
`runtime/host/Cargo.toml` already depends on `skiff-runtime-request`.

The VCP uses the production compiler and canonical publication/store/bootstrap
helpers already available to host tests.  Its fixture is the Phase 1 scalar
lane:

```text
typedJson HTTP body `2`
  -> run(value: number)
  -> exact local helper call, arithmetic and branch
  -> typedJson response `3.0`
```

It must not use the existing `rawHttp` `HttpRequest`/`HttpResponse` aggregate
wrapper.  That would make Phase 0 acceptance depend on aggregate construction
and lifecycle explicitly excluded from the Phase 1 MVP, whereas the compiler's
`typedJson` surface only requires at least one `http.body` formal.  It would
also conceal the current typed-body materialization defect rather than prove
the scalar boundary.

## Production observation contract

Add one dependency-neutral, typed contract in
`runtime/model/src/bytecode_execution_observation.rs`, re-exported from
`runtime/model/src/lib.rs`:

- `BytecodeExecutionCorrelation { router_session_id, request_id }`;
- `BytecodeExecutionObservation { correlation, ordinal, event }`;
- `BytecodeExecutionEventSink::observe(&self, observation)`;
- cloneable `BytecodeExecutionObserver`, holding the sink, correlation and one
  per-request monotonic ordinal;
- `BytecodeExecutionEvent` and the small payload enums listed below.

`observe` returns `()` and is failure-isolated: a sink failure or panic is
discarded and cannot change admission, route choice, scheduling, VM control,
terminal ownership, response bytes, or cleanup.  A no-op sink is a normal
production composition, not `cfg(test)` behavior.  Payloads contain typed
identities and bounded scalar coordinates only; they contain no image, entry,
fiber, callback, response body, mutable handle, verdict, or function capable
of resuming execution.  The correlation and ordinal are evidence metadata,
not request ownership: this decision does not claim to repair the independent
session-ownership issue described by VM-13.

`RuntimeHost` owns the sink.  At the existing wire ingress it creates one
observer from `(router_session_id, request_id)` and threads that same handle
through route admission, the request supervisor, request execution and VM.
The default host sink is implemented in
`runtime/host/src/host/bytecode_execution_observation.rs`; it projects the typed
events into the existing bounded `TelemetryProducer` without inspecting or
altering an execution result.  The projection preserves correlation, ordinal
and event name.  A host-internal VCP may install a recording implementation of
the same production sink contract on the host before ingress; this is an
observation substitution only, not an alternate executor.  There is no
`observe_for_test`, public registry getter, public spawn wrapper, or test-only
route.

The five events are:

| Event | Sole mint point | Required bounded facts |
| --- | --- | --- |
| `DeploymentImageSelected` | `BytecodeDeploymentRegistry::route`, only after the cache returns an admitted immutable `DeploymentImage` and its owner matches the requested deployment | exact `ServiceDeploymentRef`; deployment build ID derived from `image.owner().build_id()` |
| `RouteEntryPinned` | `BytecodeRoute::request_target`, only after verified entry lookup and successful `BytecodeRequestTarget::try_new_gateway`/`try_new` | image owner; ingress/operation selector; gateway key, gateway identity and observed callable role where applicable; verified function index |
| `VmFirstInstructionDispatched` | `VmFiber::dispatch_one`, once per root request, only after an opcode arm returns `Ok` | image owner; root-entry function, current function, instruction index and `Opcode` |
| `RequestTerminalClaimed` | `RequestSupervisor`, only after the winning completion claim removes the matching active row and finishes its budget | `Succeeded`, `Failed`, or `Cancelled` |
| `RequestCleanupComplete` | the host request task finalizer, after request execution/driver, target, supervised-request handle and route/image request pins have been explicitly dropped and the correlated supervisor row is absent | no synthetic counts; the event itself states request-local cleanup |

The VM event is deliberately a one-shot successful dispatch marker.  Phase 0
requires at least one actual dispatch, not per-instruction tracing; emitting at
the successful return from the selected opcode arm distinguishes dispatch from
decode or budget allocation while keeping the payload and volume bounded.
Cleanup means request-local owners and the active-supervisor row, not eviction
of the deployment image cache.  Image cache retention is expected production
behavior.

For one correlation the observer ordinals must establish:

```text
DeploymentImageSelected
  < RouteEntryPinned
  < VmFirstInstructionDispatched
  < RequestTerminalClaimed
  < RequestCleanupComplete
```

The external response frame remains the authority for `3.0`; events do not
carry or certify the result.  Missing, duplicate, out-of-order, dropped, or
wrong-correlation events fail the Gate's evidence check, but event delivery
failure never changes the request result.

This contract follows the accepted rule that production observation may be a
narrow typed event with a production owner, correlation and bounded payload,
but may not select execution or generate a verdict
(`tasks/phase-0-supplemental-closure.md:137-141`).  It also preserves the
architecture rule that tests enter the same loader/linker/verifier/VM path and
may not introduce a second evaluator (`doc/architecture/bytecode-vm.md`,
sections 2.1-2.2 and 18).

## Mandatory production repairs before the VCP can be green

These are prerequisites, not observation fields or fixture workarounds.

1. **Exact route identity and pinned metadata (VM-11).**
   `BytecodeRoute::new` currently puts
   `deployment_record.implementation.package_build_id` in `route.build_id`
   (`runtime/host/src/loader/bytecode_admission.rs:250-255`), while the image
   owner derives its build ID from
   `ServiceDeploymentRef.deployment_artifact_identity`
   (`runtime/deployment-image/src/owner.rs`).  Request envelopes and telemetry
   consequently report the wrong identity.  The route must derive deployment
   identity from `image.owner()` and must use the deployment/gateway/adapter
   facts already pinned in the admitted image rather than reopening
   `artifact_root` in `BytecodeRoute::new` and `http_adapter`.  Observation must
   not normalize or hide either mismatch.  This is the existing D0-02/VM-11
   obligation: Phase 1 must not re-read artifact storage during a request or
   substitute package identity for deployment identity
   (`dec0-architecture-decision-packet.md:23-43`; architecture review VM-11).

2. **Typed JSON body materialization.**
   `gateway_entry_arguments` currently turns every `HttpBody` into heap bytes
   (`runtime/request/src/bytecode_ingress.rs:1939-1957`), regardless of
   `HttpAdapterKind::TypedJson` and the pinned entry signature.  Therefore body
   `2` cannot enter `run(value: number)`.  The request boundary must parse the
   typed JSON body and materialize the supported scalar against the exact
   pinned verified parameter type/plan, failing closed on malformed JSON,
   arity/type mismatch or unsupported non-scalar shape.  `rawHttp` behavior is
   unchanged.  This is a narrow Phase 1 scalar boundary repair, not a new
   adapter schema or verifier/image semantic decision.

## Write ownership and ordering

The following sets are exclusive.  `D0-R` and `D0-M` land before `D0-O`; the
Proof line rebases after all three.  This avoids concurrent edits to
`bytecode_ingress.rs` and makes the two real production defects visible rather
than folding them into observability.

| Task | Exact write set | Obligation |
| --- | --- | --- |
| `D0-R` route identity/pinning repair | `runtime/host/src/loader/bytecode_admission.rs`; focused assertions in `runtime/host/src/host/request_entry/bytecode_http_tests.rs` | deployment build ID comes only from `image.owner()`; request route/adapter facts are pinned, with no post-cache artifact-root read |
| `D0-M` typedJson scalar materialization | `runtime/request/src/bytecode_ingress.rs`; focused cases in `runtime/request/tests/bytecode_request.rs` | body `2` materializes as VM `number`; malformed/wrong/non-scalar cases fail closed; rawHttp is unchanged |
| `D0-O` typed observation | `runtime/model/src/bytecode_execution_observation.rs`; `runtime/model/src/lib.rs`; `runtime/vm/src/fiber.rs`; VM call-site updates in `runtime/vm/src/fiber/projection_tests.rs` and `runtime/vm/tests/vertical.rs`; `runtime/request/src/bytecode_ingress.rs`; request call-site updates in `runtime/request/tests/bytecode_request.rs`; `runtime/host/src/loader/bytecode_admission.rs`; `runtime/host/src/host/bytecode_execution_observation.rs`; `runtime/host/src/host/mod.rs`; `runtime/host/src/host/runtime_host.rs`; `runtime/host/src/host/request_entry/assembly_wire.rs`; `runtime/host/src/host/request_entry/assembly.rs`; `runtime/host/src/host/request_entry/websocket_jsonrpc.rs`; `runtime/host/src/host/request_entry/bytecode_http_tests.rs`; `runtime/host/src/host/request_supervisor.rs` | add only the contract, production projection, propagation and five sole mint points; no result-returning hook or execution API; start only after D0-R and D0-M join |
| `P0-V-H` host VCP | Phase 0 fixture directory; `runtime/host/src/host/request_entry/phase_0_vcp_tests.rs`; module registration in `runtime/host/src/host/request_entry.rs`; remove/supersede only `runtime/request/tests/bytecode_vm_phase_0_vcp.rs` | compile/publish canonical fixture, send exact typedJson wire request through `spawn_bytecode_request`, capture raw response plus typed observations, and write no PASS verdict |
| `P0-N` proof companions | separate host request-entry negative test file and its module registration only | corrupt artifact, wrong deployment/entry and unsupported capability fail at production boundaries and have no dispatch event |

If implementing pinned adapter/route facts requires exposing an existing
read-only fact from `VerifiedLinkedBytecodeImage`, `D0-R` must first narrow its
write set in the execution map; it may not reopen linker, verifier, or image
semantics under this decision.

`D0-O` must keep `DeploymentImageSelected` and `RouteEntryPinned` in
`runtime/host/src/loader/bytecode_admission.rs`, their actual state owner.  It
must not move either mint to `assembly_wire` merely to avoid a sequential edit
after D0-R.  Its cleanup check is keyed to the matching supervised request;
global `active_count()` is not evidence that this request's row was removed.

`P0-G` consumes raw compiler/publication receipts, the wire response, and the
five correlated events.  It proves exact candidate/tree, exact published
deployment build ID, the same image owner on image/entry/dispatch, the expected
gateway identity and function, at least one actual dispatch, one successful
terminal, one later cleanup, response `3.0`, zero relevant observation drops,
and exact event cardinality/order.  Its structural reverse check must reject
direct construction/calls of linked image, verified image, executable image,
entry target, VM fiber, registry route, or event minting in the harness.

## Rejected alternatives

- Keeping the VCP in `runtime/request`: it cannot reach the host-owned registry
  without a dependency cycle or reconstructed composition authority.
- Making the registry or spawn path public, or adding a test-only host runner:
  either enlarges execution authority solely for proof.
- Calling `bytecode_deployments.route` to prewarm the host test: it bypasses the
  highest production ingress and can manufacture the fact the VCP must prove.
- Hand-building a deployment image, verified entry, request target or fiber:
  explicitly forbidden by the Phase Contract
  (`tasks/phase-0-supplemental-closure.md:50-58,104-112`).
- Inferring VM dispatch from `3.0`, allocated fuel, logs or a terminal alone:
  none is minted at successful opcode dispatch.
- Using rawHttp aggregates to avoid typedJson materialization: it expands the
  Phase 1 surface and conceals the boundary defect.
- Reporting the package build ID as if it were deployment/image identity: it
  contradicts D0-02 and VM-11.
- A sink that returns `Result`, chooses routes, resumes work, owns an image, or
  emits PASS: it is an execution or verdict authority, not observation.
- Starting an external router/runtime process: it is a valid later system
  regression, but is larger than the existing host-internal production
  composition needed for this Phase 0 VCP.

## Consumers and proof obligations

- `D0-R` and `D0-M` consume the two prerequisite findings and must land focused
  negative tests before `D0-O`/Proof can claim exactness.
- `D0-O` consumes this event table literally; adding richer tracing requires a
  separate decision and is not Phase 0 work.
- `P0-V-H` owns only fixture/harness/raw capture and cannot modify production.
- `P0-G` owns verdict aggregation and durable receipts; the harness never
  writes `status: pass`.
- `P0-N` proves fail-closed scenarios do not emit a successful route/entry or
  dispatch sequence.
- An independent reviewer must verify the observer is failure-isolated and
  that every event has exactly one production mint point.  Fresh Acceptance
  must run the canonical Gate from a clean frozen candidate, as required by
  `tasks/phase-0-supplemental-closure.md:149-161` and
  `tasks/phase-0-recovery-execution-map.md:88-95`.

No verifier disposition, deployment-image representation, scheduler semantics,
request ownership identity, or user-visible support beyond the accepted
typedJson scalar lane is decided here.
