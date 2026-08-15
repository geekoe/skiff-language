# DEC6: cross-owner kernel, memory ledger, and service child seam

> Status: architecture decision for Phase 6 central F6/K6/X6 seams
>
> Scope: owner/heap/root/budget/memory authority split, flat child lifecycle, service boundary facts and dispatch
>
> Read inputs: Phase 6 contract/MAP plus current scheduler, request, VM, loader, linker, artifact-model and boundary seams

## 1. K6 verdict: one flat scheduler with one heap carrier per execution unit

Keep the existing `BytecodeScheduler` and `FlatTrampoline` as the only execution loop. Do not introduce a service child loop, a second pending registry, a second budget, or a second owner inventory. The minimal change is to make the trampoline carry one movable heap carrier per execution unit and to let the child executor return both a child unit and the exact continuation boundary that materializes child results into the parent heap.

Recommended scheduler-neutral types:

```text
ChildHeapCarrier
  heap: Box<dyn VmHeap + Send>
  domain: HeapDomainId
  epoch: HeapEpoch
  memory_lease: MemoryLease
  staging: BoundaryStaging
  heap_owner_lease: ChildHeapOwnerLease

BytecodeChildStart<U>
  unit: U
  resume: U::ResumeToken
  child_heap: ChildHeapCarrier
  finish: Box<dyn ChildFinish<U>>

trait ChildFinish<U>
  fn finish(
    self,
    child_result: U::RootResult,
    child_heap: &mut ChildHeapCarrier,
    parent_heap: &mut dyn VmHeap,
    budget: &mut dyn VmBudget,
  ) -> Result<U::ResumeOutcome, ChildFinishError<U>>
```

`FlatTrampoline` becomes:

```text
FlatTrampoline<U, R>
  active: U
  active_heap: ChildHeapCarrier
  blocked: Vec<BlockedUnit<U, R>>

BlockedUnit<U, R>
  parent: U
  parent_heap: ChildHeapCarrier
  resume: R
  owner_lease: Option<ChildOwnerLease>
```

Run and resume rules:

- `run_segment` always runs the active unit against its own `ChildHeapCarrier`; the parent heap is never handed to the child.
- Entering a child moves the current unit and its heap into `BlockedUnit`, then installs the child unit and child heap as active. This is the atomic owner-bundle publish point.
- Child completion returns the completed child heap with the parent trampoline. The scheduler invokes `ChildFinish` before resuming the parent. `ChildFinish` allocates parent-heap values from exact boundary facts, binds them through `VmOwnedValues::try_from_resume` or the equivalent throw path, releases child-owned values on the child heap, and then drops the child heap.
- Actual `Pending` moves the entire `SuspendedTrampoline`, including active and blocked `ChildHeapCarrier`s, into the existing Phase 4 pending owner. The child heap stays inside the suspended chain, not in a separate registry, sidecar map, or host task.
- Request terminal keeps the same suspended/failure owner graph; cleanup releases child terminal escrows on the correct child heap before dropping that heap, then releases parents reverse order.

`ChildHeapCarrier` should implement `VmRootSource` by visiting any published or retained staging/terminal roots it owns. The fiber roots themselves remain inside `VmFiber`; the carrier is what keeps the corresponding heap and owner lease alive.

Owner inventory: extend the existing `RequestExecutionOwnerInventory`, not a new registry, with a `ChildHeap` domain and `ChildHeapOwnerRegistration`/`ChildHeapOwnerLease`. `ChildHeapCarrier` holds the lease. The frozen snapshot should add child-heap counts alongside pending/resource/child counts.

Domain identity: stop using the global 8-bit domain counter in `RequestVmHeap`. Mint `HeapDomainId` from the request-level `RequestMemoryLedger`, and change `VmHandle` encoding from 8-bit domain + 56-bit serial to a wider request-scoped domain or equivalent monotonic non-wrapping identity. This is a K6 `vm_value.rs`/`vm_heap.rs` change and prevents 256-heap reuse.

## 2. Request-shared vs owner-local authority

Current construction:

- `RequestExecutionContext::create` opens the one owner inventory, resource table and scheduler ports.
- `start_bytecode_request` constructs the root `RequestVmHeap`.
- `execution_budget.attach_vm()` constructs the one VM budget adapter.
- `drive_runtime_bytecode_request_controlled` builds `RequestPendingRuntime`, pending registry/wake queue and `BytecodeSchedulerPorts`.

Recommended split:

Request-shared authority:

- one `RequestExecutionContext` / `RequestExecutionOwnerInventory`
- one `RequestResourceTable`
- one `Arc<ExecutionBudget>` and one attached `BytecodeVmBudget`
- one `RequestMemoryLedger`
- one `RequestPendingRegistry` / wake queue
- one `BytecodeSchedulerPorts` / child mux

Owner-local authority:

- one `VmFiber` / execution unit per owner
- one `RequestVmHeap` per owner, wrapped in `ChildHeapCarrier`
- one `HeapDomainId` + epoch per heap
- one `MemoryLease` per heap
- owner-local boundary staging, callback/capability staging and cleanup retention

Seam for child heap creation:

```text
RequestMemoryLedger::mint_child_heap(owner, limits)
  -> (HeapDomainId, MemoryLease)

trait BytecodeChildHeapFactory
  fn create(
    owner: &DeploymentOwnerIdentity,
    limits: RequestHeapLimits,
    resources: RequestResourceTable,
    ledger: Arc<RequestMemoryLedger>,
  ) -> Result<ChildHeapCarrier, BytecodeChildError>
```

`BytecodeRequestExecutionInput` or the request-side `RequestPendingRuntime` must carry `Arc<RequestMemoryLedger>` and the child factory. `RequestVmHeap` gets a production constructor that binds the shared resource table, ledger and newly minted domain/epoch. Cleanup is RAII plus explicit terminal release: `ChildHeapCarrier` releases its heap owner lease and memory lease on drop, but only after the request driver has released every terminal escrow whose slots belong to that heap. `BytecodeRequestRetention` must become owner-ordered (`Vec<ChildHeapCarrier>` leaf-first) instead of one root heap.

## 3. F6 verdict: existing facts are close, but a minimal schema extension is required

Existing facts already cover:

- `ServiceCallRef` and `BytecodeRelocation::ServiceOperationRef`
- `ServiceContract.operations[ContractOperationId]` -> `BoundaryOperationDescriptor`
- `BoundaryOperationContract` parameters, return, stream, callbacks and effect guarantee
- `BoundaryValuePlan` carrier/encoding/owner/lifetime
- `PackageArtifact.boundary_projections` and `CallableSemanticFacts`
- `ServiceDeployment.operation_bindings` contract operation -> provider callable
- hydrated dependency contracts and `DeploymentExecutionImage.dependency_slots`

Still missing before `link_service_operations` can build a safe `LinkedServiceOperationTarget`:

1. Ordinary-error boundary plan. `BoundaryOperationContract` has no error contract. Current language documentation deliberately does not publish a static throw set, so the exact plan cannot be a static exhaustive type list. The minimal fact is a `BoundaryErrorPlan` that carries the exact fallback identity/policy, the runtime public-schema check rule, carrier, transfer/drop and source attribution, while leaving the concrete error type dynamic.
2. Per-value transfer/drop/source facts. `BoundaryValuePlan` has owner/lifetime/carrier/encoding but not copy/move/drop. Add `BoundaryTransfer` (`Copy`/`Move`) and `BoundaryDropPlan`, or an equivalent compiler-emitted boundary transfer fact, so the materializer can know when to release the source owner.
3. Cross-image canonical plan. `LinkedCallableSignature` is image-local (`TypeIndex`, `LinkedValueTransferPlan`). Cross-owner materialization needs a canonical `ContractTypeRef`/`PackageSchemaTypeId` plus runtime-tag facts for caller and provider, not an image-local index guessed by shape or name. Add a `LinkedServiceBoundaryPlan` table with exact argument/result/error/stream-item/callback facts.
4. Exact stream item and callback plans if those surfaces are accepted in service lane; otherwise they must fail at admission before an image is built.

Recommendation: Phase 6 needs a minimal artifact schema extension, owned by F6, not a linker-side reconstruction. Minimal fields:

```text
BoundaryErrorPlan
  fallback: BoundaryValuePlan
  policy: BoundaryErrorPolicy
  transfer: BoundaryTransfer
  drop: BoundaryDropPlan
  source: ValueProvenance

LinkedServiceBoundaryPlan
  arguments: [LinkedServiceBoundaryValue]
  results: [LinkedServiceBoundaryValue]
  error: LinkedServiceBoundaryValue
  stream_item: Option<LinkedServiceBoundaryValue>
  callbacks: LinkedServiceCallbackPlan
```

`LinkedServiceOperationTarget` should retain the existing key/operation/protocol identity and add this boundary plan. It must not embed a provider deployment; provider resolution remains a runtime boundary action per the release-pointer model.

## 4. X6 verdict: provider child dispatch through the RuntimeHost composition seam

Service child dispatch should be one branch in the new request child mux, implemented under X6 `bytecode_children/service.rs` and injected through the existing `RuntimeHost` composition.

Flow:

1. Scheduler sees `ChildTarget::Service(ServiceOperationIndex)` in a `ChildInvocation`.
2. Caller image `service_operations()` yields `LinkedServiceOperationTarget`; caller image `dependency_slot(key)` yields the exact contract dependency slot.
3. Host-owned service child resolver resolves `(profile, serviceId, version)` through the existing release-pointer seam to an exact `ServiceDeploymentRef`, loads/pins the provider `DeploymentExecutionImage` through `BytecodeDeploymentRegistry`, checks `expected_protocol_identity`, and selects `operation_entry(contract_operation_id)`.
4. Child heap factory creates the provider `ChildHeapCarrier` under its own domain/epoch with the shared request resource table and `RequestMemoryLedger`.
5. X6 materializer copies caller arguments from the caller heap into the provider heap using the boundary plan, then calls `Vm::start(provider_entry, provider_args, ...)`.
6. `BytecodeChildExecutor::execute_child` returns `BytecodeChildStart<VmFiber>`; the scheduler enters the provider fiber with its own heap.

Sync `Ready`: the provider fiber completes synchronously, `ChildFinish` materializes result or ordinary throw into the caller heap, the child heap is dropped, and the parent resumes exactly once. No pending owner is minted.

Actual `Pending`: the provider fiber returns `Park`; the existing scheduler path suspends the whole trampoline, including the provider `ChildHeapCarrier`, and publishes through the existing `RequestPendingRegistry`. Wake/claim restores the same child heap and provider fiber; there is no second pending authority.

Ordinary throw: `VmCompletion::Thrown` is materialized through the error plan into a caller-owned `VmOwnedException`, then returned as `ResumeOutcome::Throw`. The caller catch identity is preserved; the child heap is released after its owned error/staging roots are gone.

Cleanup: provider resolution failure, argument materialization failure, child start failure, request terminal, cancel, deadline and duplicate wake all return the still-owned parent invocation, child heap, staging, memory lease and image pin to the existing owner-bearing failure/retention path. The parent is never marked blocked before the child bundle is fully constructed.

Provider image lazy-loading can make resolution genuinely asynchronous. Keep that on the same flat scheduler by allowing a child-handoff result:

```text
BytecodeChildHandoff<U>
  Ready(BytecodeChildStart<U>)
  Pending(U::PendingOperation)
```

When resolution must wait, the child mux parks through the request pending registry and, on wake, produces the `BytecodeChildStart` and enters the child. Do not block inside the scheduler or create a second child loop.

## 5. DECISION NEEDED

Only one item genuinely needs user/product judgment from this research:

1. Service ordinary-error materialization policy.

   Existing language and service API documentation say service operations do not publish a static throw set and arbitrary user types may be thrown, with a bounded dynamic public-schema check and `std.service.InternalError` fallback. The Phase 6 Contract asks for a compiler-emitted ordinary-error plan.

   Recommended resolution: keep the existing dynamic error policy and make `BoundaryErrorPlan` a bounded error-channel policy with exact fallback identity, transfer/drop and runtime public-schema check. This avoids changing the language and matches `doc/reference/runtime.md`.

   If instead the product wants a fully static per-operation error plan, that requires an explicit language/API contract amendment to declare a closed, schema-public throw set. Local code cannot decide that because it changes user-visible language semantics.

No other research finding requires user judgment: the flat trampoline, shared budget/memory/owner inventory, provider release-pointer resolution, per-owner heap, and same-Runtime child path are all determined by the existing Contract, MAP and runtime documentation.
