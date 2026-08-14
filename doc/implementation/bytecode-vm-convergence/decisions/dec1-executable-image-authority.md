# DEC1-K1: deployment executable-image authority

> Status: corrected decision; Amendment r2 hard cut authorized; no architecture-review gate
>
> Input: `76a0a1894d76e24d2f1118a18c209eb5b6f0e50d`
>
> Scope: K1 executable-image construction, publication, entry pin and VM input only

## Decision

The sole bytecode execution authority is one immutable
`DeploymentExecutionImage` for one exact deployment owner/build. Its identity
is `DeploymentOwnerIdentity`; the cache has one slot per deployment build ID
with the existing full-owner conflict fence. Route, operation, gateway and
callable-role identity are not cache-key components and cannot cause a second
image allocation for the same build.

The same `Arc<DeploymentExecutionImage>` is stored by the cache, pinned by an
image-owned entry lookup, carried through the request boundary and retained by
the VM. There is no `VerifiedLinkedBytecodeImage` inside an outer
`DeploymentImage<P>`, no verification seal and no independently pairable
image/entry/request-target authority.

The sole public production mint is deployment-wide and atomic:

```rust
pub fn link_deployment_execution_image(
    hydrated: HydratedDeploymentBytecode,
    limits: &DeploymentExecutionLimits,
) -> Result<DeploymentExecutionImage, DeploymentExecutionImageError>;
```

It has no route selector or requested-entry parameter. It consumes the exact
hydration, derives the deployment's complete canonical publication-root set,
links the union of those roots, runs K0B containment, invokes the independent
post-link verifier, materializes immutable image facts and returns only the
finished value. Link candidate, verification scratch, `ExecutableFacts`, heap
builder and image parts remain construction-local. There is no public parts
constructor, builder, `From` conversion, `Default`, test constructor or
partially executable result.

`skiff-runtime-linker` owns this type and atomic function. The existing
`skiff-runtime-bytecode-verifier` crate may remain as the implementation home
of the independent post-link algorithm; K1 does not delete it merely to rename
the boundary. Its transitional `ExecutableFacts` is opaque, has no public
constructor or execution/entry API, is not re-exported to host/request/VM and
has exactly one non-test consumer: the atomic linker function. If crate
visibility requires an integration symbol, source-boundary checks still
enforce that sole consumer. `ExecutableFacts` is moved into the final image
fields immediately and is never cached or retained as a parallel authority.

## Canonical publication roots and K0B containment

`canonical_roots` means the canonically ordered union of every public root of
the deployment, not the root selected by the current request. Exactness applies
to each identity and edge in this union; it does not imply a per-entry image.

| Declared surface | Phase 1 root behavior |
| --- | --- |
| service operation | every exact `ContractOperationId` binding is one publication root, ordered by the typed ID; all are linked and gated |
| unary HTTP gateway | one protocol-aware entry root whose callable bundle is ordered `Guard -> Pre -> Handler`; Phase 1 accepts only the handler-only shape |
| HTTP `Guard` or `Pre` | declaring either makes that public HTTP bundle unsupported and rejects the whole deployment before publication; neither role is exposed as a standalone root or lookup |
| WebSocket `CloseHandler` | belongs only to a distinct future WebSocket publication root; it is never folded into an HTTP root and the entire WebSocket root class is disabled in Phase 1 |
| Actor create/method, task, stream or another disabled public root class | typed unsupported-root failure before image publication; it is not silently omitted from a deployable public surface |
| raw private function | not a publication root; if unreachable from the union it is absent from the linked function closure and an unsupported semantic capability in it does not poison publication |
| private function reachable from any publication root | included once in the union closure and fully gated; unsupported facts reject the whole deployment |

For each exact HTTP gateway entry, root discovery resolves the canonical
ingress binding, gateway entry identity, adapter plan and callable bundle as
one unit. The order above is part of the protocol root contract, even though
Phase 1 rejects any entry that declares `Guard` or `Pre`. K1 must not represent
those roles as independent cache entries or allow a caller to select them.
`Handler` is the only Phase 1 HTTP VM entry. `CloseHandler` can be admitted only
after a later WebSocket decision enables that separate root class.

K0B evaluates the reachable union. Therefore unsupported code reachable from
operation B rejects the single deployment image even if a request later asks
for operation A. This is intentional deployment publication atomicity. An
unsupported raw private function unreachable from every publication root is
not linked and does not fail the semantic allowlist. Structural corruption is
different: bounded artifact validation still examines the opaque artifact and
may reject malformed bytes or indices even in an unreachable function.

Package-global constant authority remains deployment-wide. All admitted
constant roots and the graph/table rows required to construct the one constant
heap participate in preflight, limits and deterministic materialization. K1
migrates the existing linker constant and multi-package tests to the full
deployment constructor; it does not replace this with route-enumerated
constant preflight.

### Corrected K0B acceptance

K1 may consume K0B only with these assertions:

1. the root set contains every exact operation ID and every enabled
   protocol-aware publication root exactly once in canonical order;
2. an unsupported fact reachable from any public root rejects the deployment,
   including when a different supported entry would have been requested;
3. unsupported semantic facts in a raw private function unreachable from all
   publication roots do not enter the closure, while malformed artifact
   structure still fails at structural admission;
4. HTTP gateway closure is one ordered `Guard -> Pre -> Handler` bundle;
   Phase 1 rejects declared guard/pre and exposes no standalone role root;
5. `CloseHandler` is never an HTTP root, and WebSocket/Actor/task/stream root
   classes remain typed disabled failures rather than ignored declarations.

The old acceptance statement that an unrelated unsupported public root must
not affect a requested supported entry is withdrawn. It would require
per-entry images and contradict the deployment build/cache authority decided
here.

## Image-owned exact entry pins

Entry selection happens after `get_or_load` returns the single deployment
image. The route owner must still supply exact typed identity:

```rust
pub enum DeploymentEntrySelector {
    Operation {
        contract_operation_id: ContractOperationId,
    },
    HttpGateway {
        ingress: IngressSelector,
        gateway_entry_identity: GatewayEntryIdentity,
    },
}
```

`DeploymentExecutionImage::operation_entry(&Arc<Self>,
&ContractOperationId)` and the exact HTTP gateway lookup are the only entry
mints. They return an opaque `DeploymentExecutionEntry` containing the same
image `Arc` plus a private image-local entry index/kind/signature. It has no
public fields, constructor, `From` implementation or API that accepts an image
and a separately supplied function index.

Operation lookup deletes the unit `BytecodeRouteSelector::Operation` and
`operation_entry_ids().next()`. A route without a typed
`ContractOperationId`, or with an ID absent from the image map, fails after
load and before VM start; it never selects by declaration order, cardinality or
string label. This exact selector does not change the cache key and cannot
trigger relinking.

HTTP lookup exact-matches ingress, image-owned canonical binding, gateway entry
identity and the admitted handler-only bundle. The caller does not supply a
callable role. No API can mint `Guard`, `Pre` or `CloseHandler` as a Phase 1
entry. Future protocols must add protocol-specific bundle execution without
changing the one-image-per-build rule.

`Vm::start` consumes the opaque entry pin. The VM obtains its root function,
signature and image through that pin and retains the same `Arc` for the fiber,
continuations and resume carriers. The pin is a capability into the sole image,
not a second program type.

## Final image contents and fact owners

`DeploymentExecutionImage` is one closed immutable allocation with private raw
link state and narrow VM-facing accessors:

| Final fact | Sole producer and rule |
| --- | --- |
| exact owner/build, package/deployment closure | loader hydration; atomic constructor consumes it and exact-checks all package/deployment references |
| schema/ISA and six registry pins | structural admission/compiler-owned typed records; copied exactly, never reread from ambient registries |
| canonical publication-root inventory and entry maps | deployment linker from exact typed operation/gateway/protocol bindings; all roots in the one image |
| reachable functions, relocations, frame/type/shape/target tables | deployment linker over the union closure; raw unreachable private functions are excluded |
| dependency slots | exact hydration distilled once during construction |
| constant heap | atomic constructor from linker-resolved deployment-wide constant authority after bounded preflight and verifier correspondence checks |
| callable effect/lifecycle pins | source/compiler-owned typed facts carried through artifact and linker; verifier may reject mismatch but cannot derive replacements |
| immutable statement schedule and verified execution bounds | post-link verifier's `ExecutableFacts`; schedule is rebuilt only from admitted typed statement/source rows plus the fingerprinted opcode charging contract |
| root entry pins | image-owned exact lookup after publication; no caller-minted index or image/entry pairing |

Host, request, VM, scheduler and tests cannot obtain
`HydratedDeploymentBytecode`, `LinkedBytecodeCandidate`, image parts or
`ExecutableFacts` from the final image. VM accessors expose only the exact
distilled functions/tables/constants/schedule/effect facts needed by execution;
all current `candidate()` escape paths are removed.

## Independent thin verifier

The canonical post-link verifier remains inside the atomic transition and does
not trust linker summaries. For Phase 1 it independently and boundedly checks:

- CFG instruction boundaries and branch targets;
- operand-stack underflow, merge height and computed maximum depth;
- frame/slot bounds, slot liveness and parameter/result joins;
- function/table/type/shape/constant indices and exact local-call
  target/kind/arity/signature correspondence;
- absence of unresolved relocation, unresolved `TypeParam` and duplicate or
  missing image-local rows;
- correspondence of resolved constant rows with the materialized heap plan;
- effect-pin consistency for the already linked union, including no reachable
  Phase 1 Pending/effect capability;
- exact typed statement-row coverage and canonical charging placement.

Its only positive result is internal `ExecutableFacts`. The verifier is the
sole producer of the immutable statement schedule: it combines admitted typed
source/statement rows with the pinned opcode contract and CFG. That permitted
schedule construction is not source reconstruction. Missing, duplicate or
inconsistent rows fail; the verifier does not guess source placement.

The thin stage may not:

- read syntax, initializer text, File IR, release pointers or ambient artifact/
  registry state;
- infer source type, shape, lifecycle, target kind or ABI from names, strings or
  opcode spelling;
- repair linker joins through type equivalence or normalize an unknown target;
- derive callable effects by opcode scan to replace a missing source-owned pin;
- synthesize registry descriptors, adapter identity, dependency facts or entry
  maps;
- choose, omit or widen publication roots;
- return a seal, verified program, entry, heap wrapper or executable image.

A verifier error aborts the constructor. `ExecutableFacts` never reaches the
cache independently and cannot be used to start a VM.

## Sole producers and consumers

| Value / transition | Sole producer | Allowed consumers |
| --- | --- | --- |
| exact hydration | deployment loader from the selected release/deployment record | atomic image constructor only |
| canonical publication-root set | deployment linker from typed hydration | closure discovery, K0B gate and entry-map construction inside the same call |
| raw linked candidate/image parts | atomic constructor's linker internals | retained verifier and materializers during that call only |
| `ExecutableFacts` | independent post-link verifier | atomic image constructor only |
| `DeploymentExecutionImage` | `link_deployment_execution_image` | deployment cache; test-runner/package-test may construct and inspect only through final accessors |
| cache publication | deployment cache completion after exact owner equality | later routes for that same full owner/build |
| `DeploymentEntrySelector` | host route admission from typed request/release/deployment facts | lookup on the already loaded image only; never cache/link input |
| `DeploymentExecutionEntry` | methods on the loaded image `Arc` | host route, request boundary and `Vm::start` |
| VM program allocation | the `Arc` inside the image-owned entry pin | VM fiber and VM-owned continuation/resume carriers |

The cache coordinates storage and single-flight loading; it is not an image
producer. Routes do not link. Request/VM/scheduler do not select raw function
indices. Phase 0/1 proof code enters only through
`RuntimeHost::spawn_bytecode_request` and cannot call the constructor.

`test-runner/src/runtime_execution.rs` keeps its deployment prepublication
check, but changes from raw `link_deployment` to the same full-deployment
`link_deployment_execution_image` used by production and discards only a
complete final image. It cannot enumerate operation/gateway roots or run a
weaker link-only preflight. `runtime/package-test` follows the same rule, then
uses image-owned exact entry lookup for the test invocation.

## Public surface deletion

K1 deletes, rather than aliases or deprecates, these execution-construction
surfaces:

- `verify`, `VerifiedLinkedBytecodeImage`, `VerificationSeal`,
  `SealedDeploymentFacts` and verified-image constructors/accessors;
- `DeploymentImage<P>`, `DeploymentProgramFacts`,
  `DeploymentImage::try_new` and the bytecode use of
  `PinnedProviderImage`;
- `DeploymentProgramEntry`, `PinnedDeploymentEntry::try_new` and all APIs that
  accept a separately supplied image and entry;
- `BytecodeRequestTarget::{try_new, try_new_gateway}` and `VerifiedVmEntry`;
- public production `link_deployment` returning a raw candidate; linker unit
  tests migrate to the atomic final constructor or private module seams, while
  test-runner/package-test must use the production constructor;
- final-image `candidate()`, `program()`, hydration and raw-parts accessors;
- the unit operation selector and every `.next()`/single-entry fallback.

The retained verifier crate/type vocabulary is not a second image. Its old
verified entry maps move to linker/image-owned entry maps; its statement
schedule remains verifier-produced but is embedded in the single final image.
No old/new cache, compatibility alias, feature-gated bypass or `cfg(test)` image
constructor is permitted.

## Cache publication and failure semantics

The production cache remains deployment keyed:

```text
DeploymentArtifactIdentity
  -> exact DeploymentOwnerIdentity
       -> Loading(attempt) | Loaded(Arc<DeploymentExecutionImage>)
```

There is no operation, gateway or role level below the owner slot. Concurrent
requests for different entries of one build join the same attempt and then
perform independent image-owned lookups on the returned same `Arc`.

To avoid a crate-dependency cycle, the cache may remain generic over a narrow
owner-bearing `DeploymentCacheValue` trait and store `Arc<P>` directly. That
trait does not expose program facts or construct/wrap a value; production has
exactly one implementation, `DeploymentExecutionImage`. It replaces the broad
`DeploymentProgramFacts`/`DeploymentImage<P>` pairing rather than preserving
it under another name.

Hydration, full-root linking, K0B gating, post-link verification, schedule and
heap materialization occur in loader-task locals. `Loading`, raw link parts and
`ExecutableFacts` are never returned by `loaded` or snapshots. Only
`Ok(image)` with `image.owner() == requested_owner` may atomically replace
`Loading` with `Loaded`.

Resolver/hydration/link/K0B/verifier/materialization error, panic, cancellation,
attempt-state loss or output-owner mismatch clears the build slot and publishes
no image. Joined waiters observe the same attempt ID and shared failure. A later
request may retry from the empty slot; it cannot resume partial construction.
Caller cancellation does not expose partial state: the owned background task
either publishes one complete image or records failure. A failure in any public
root rejects the complete build and cannot leave a smaller entry-specific image
cached.

## Atomic migration order

K1 lands as one hard-cut commit; intermediate local compile states are not
supported publication states.

1. Freeze the corrected canonical publication-root inventory and K0B tests:
   all public roots unioned, handler-only HTTP accepted, disabled public root
   classes rejected and unreachable raw private functions excluded.
2. Refactor the retained verifier to return construction-only
   `ExecutableFacts`, keeping it the sole statement-schedule producer while
   deleting the seal/verified-image/verified-entry outputs.
3. Add `DeploymentExecutionImage` and the no-selector atomic constructor. Move
   owner, dependency, entry-map, constant-heap and linked closure assembly into
   that one transition; make raw linking non-production-private.
4. Change deployment-image cache storage from an outer generic image wrapper
   to direct `Arc<DeploymentExecutionImage>` while preserving the build-ID/full
   owner fence and complete-or-none attempt protocol.
5. Add opaque image-owned operation and HTTP handler entry lookup. Change route
   operation input to exact `ContractOperationId`; remove role-selectable HTTP
   lookup and all first-entry behavior.
6. Move host, request, VM, scheduler and continuation carriers to the same
   image `Arc` through the opaque pin; replace every raw `candidate()` access
   with a narrow final-image accessor.
7. Move package-test, VM/request fixtures, linker constant/deployment tests and
   `test-runner/src/runtime_execution.rs` to the full production constructor;
   no test enumerates ad hoc entry roots.
8. In the same commit remove old constructors, wrappers, aliases and direct
   crate dependencies, then run reverse-search and focused acceptance tests.

## K1 exact write set

The K1 writer owns only the paths below. The directory entries name exact
existing closed subtrees. A required change elsewhere returns to Design/
integration before editing; K1 does not change compiler, artifact/wire schema,
Phase Gate or Proof verdict code.

| Area | Exact paths |
| --- | --- |
| dependency declarations | `Cargo.toml`; `Cargo.lock`; `runtime/linker/Cargo.toml`; `runtime/bytecode-verifier/Cargo.toml`; `runtime/deployment-image/Cargo.toml`; `runtime/host/Cargo.toml`; `runtime/request/Cargo.toml`; `runtime/vm/Cargo.toml`; `runtime/package-test/Cargo.toml`; `test-runner/Cargo.toml` |
| final image, deployment-wide linker and root closure | `runtime/linker/src/lib.rs`; all existing files under `runtime/linker/src/bytecode/`; new `runtime/linker/src/bytecode/execution_image.rs`; new files under `runtime/linker/src/bytecode/execution_image/` |
| mandatory linker tests called out explicitly | `runtime/linker/src/bytecode/tests.rs`; `runtime/linker/src/bytecode/tests/deployment.rs`; `runtime/linker/src/bytecode/tests/constants.rs`; `runtime/linker/src/bytecode/tests/constants/multi_package.rs`; existing fixture files under `runtime/linker/src/bytecode/tests/fixtures/`; new `runtime/linker/src/bytecode/tests/execution_image.rs` |
| retained thin verifier and its focused tests | `runtime/bytecode-verifier/src/lib.rs`; all existing files under `runtime/bytecode-verifier/src/` (retain the crate; replace broad outputs with internal `ExecutableFacts`) |
| direct deployment-build cache | `runtime/deployment-image/src/lib.rs`; `runtime/deployment-image/src/attempt.rs`; `runtime/deployment-image/src/cache.rs`; `runtime/deployment-image/src/load.rs`; `runtime/deployment-image/src/owner.rs`; `runtime/deployment-image/src/state.rs`; `runtime/deployment-image/src/image.rs` (delete); `runtime/deployment-image/src/entry.rs` (delete); `runtime/deployment-image/src/pin.rs` (delete); all existing files under `runtime/deployment-image/src/tests/` |
| host load/route | `runtime/host/src/loader/bytecode_admission.rs`; `runtime/host/src/host/request_entry/assembly_wire.rs`; `runtime/host/src/host/request_entry/bytecode_http_tests.rs` |
| request consumers | `runtime/request/src/lib.rs`; `runtime/request/src/bytecode_ingress.rs`; `runtime/request/src/failure_projection.rs`; `runtime/request/src/failure_projection/tests.rs`; `runtime/request/tests/bytecode_request.rs` |
| VM/scheduler consumers | `runtime/vm/src/lib.rs`; `runtime/vm/src/admission.rs`; `runtime/vm/src/budget.rs`; `runtime/vm/src/control.rs`; `runtime/vm/src/error.rs`; `runtime/vm/src/fiber.rs`; `runtime/vm/src/fiber/entry_admission.rs`; `runtime/vm/src/fiber/entry_admission/tests.rs`; `runtime/vm/src/fiber/projection_tests.rs`; `runtime/vm/src/fiber/tests.rs`; `runtime/vm/src/frame.rs`; `runtime/vm/src/projection.rs`; `runtime/vm/src/statement.rs`; `runtime/vm/src/statement/tests.rs`; `runtime/vm/tests/vertical.rs`; `runtime/scheduler/src/stream_driver.rs` |
| production-shaped test consumers | `runtime/package-test/src/lib.rs`; `test-runner/src/runtime_execution.rs`; `test-runner/src/runtime_execution/tests/orchestration.rs`; `test-runner/src/runtime_execution/tests/support.rs` |
| dependency-boundary registries | `scripts/check-runtime-crate-dag.mjs`; `scripts/lib/verify-rust-subjects.mjs` |

The linker/verifier directory ownership is limited to the image protocol,
corrected root closure, verifier output and required test migrations. It does
not authorize Phase 2 aggregate, exception, Pending, stream, task, Actor,
resource or effect execution work.

## K1 acceptance tests

K1 is accepted only if focused tests and structural review prove:

1. one deployment with operations A and B constructs and caches one image;
   exact lookups for A and B return pins holding the same `Arc`, and neither
   lookup relinks or changes the cache key;
2. missing/unknown operation ID fails without `.next()` or single-entry
   fallback, while a valid exact `ContractOperationId` selects only its map row;
3. unsupported code reachable only from public operation B rejects the whole
   deployment even when A would otherwise execute; an unsupported semantic
   opcode in an unreachable raw private function is excluded from the closure;
4. malformed structure in any admitted artifact still fails before linking,
   independently of function reachability;
5. handler-only unary HTTP root succeeds; declaring guard or pre rejects the
   entire HTTP bundle, no standalone role lookup exists, and WebSocket close/
   Actor/task/stream roots remain disabled failures;
6. canonical root order and resulting function/entry maps are deterministic
   across repeated construction, including all exact operation IDs and gateway
   entries;
7. linker constant and multi-package tests exercise the final constructor and
   prove deployment-wide preflight, limits, deterministic heap contents and no
   partial heap publication;
8. the verifier catches CFG/stack/slot/index/call/unresolved-relocation defects,
   sole-produces the statement schedule from typed rows/opcode contract, and
   cannot reconstruct missing effects, registry pins, entries or source rows;
9. two concurrent entry routes join one deployment attempt and receive the
   same complete image; resolver/link/K0B/verifier/heap error, panic,
   cancellation, attempt loss and owner mismatch expose no loaded or partial
   image, and retry starts fresh;
10. host -> request -> `Vm::start` retains the same image allocation through an
    image-minted pin; request/VM/scheduler have no raw hydration/candidate or
    image/entry pairing API;
11. package-test and `test-runner/src/runtime_execution.rs` call the sole full
    production constructor and never enumerate requested roots or call raw
    `link_deployment`;
12. reverse searches find no `VerifiedLinkedBytecodeImage`, `VerificationSeal`,
    `DeploymentImage<...>`, `PinnedDeploymentEntry::try_new`,
    `BytecodeRequestTarget::try_new*`, `VerifiedVmEntry`, final-image
    `candidate()`/`program()`, unit operation selector, operation `.next()`,
    per-entry cache branch or second executable-image type.

No Cargo command was run for this Design receipt. K1 owns the focused commands;
its tests do not constitute Phase 1 Acceptance.

## Rejected alternatives

- **Per-request/per-entry image or cache key.** It permits multiple immutable
  programs for one build and lets one public root escape deployment publication
  failure; this is the withdrawn version of this receipt.
- **Deployment cache plus secondary root-image cache.** Two cache levels still
  create multiple execution authorities and make failure publication partial.
- **`VerifiedLinkedBytecodeImage` inside `DeploymentImage<P>`.** Renaming either
  layer preserves the current dual-image boundary.
- **Verifier-produced verified program/seal.** A verifier-owned program consumed
  by VM is a second image. `ExecutableFacts` is construction-only and cannot
  select or execute an entry.
- **Route-selected linking after load.** It makes request order affect image
  contents and violates one build/one slot.
- **Standalone gateway role entries.** HTTP guard/pre/handler are one ordered
  protocol bundle; Phase 1 rejects guard/pre instead of publishing role shards.
- **Treating WebSocket close as an HTTP role.** `CloseHandler` belongs to a
  separate, currently disabled WebSocket root class.
- **Verifier reconstruction from source or registries.** It creates a competing
  semantic producer and can hide missing typed pins; schedule construction from
  admitted typed rows and pinned opcode contract is the sole explicit exception.
- **Raw linker-only test-runner preflight.** It tests weaker authority than
  production and can pass a deployment whose verifier/image construction fails.
- **Compatibility aliases, old/new caches or test-only image builders.** They
  prevent mechanical proof of a single mint and are unnecessary for an
  unpublished language.

## Historical review questions (non-blocking)

The following questions were used by the earlier process. They remain historical design prompts, not an independent
review/PASS prerequisite. Amendment r2 supersedes question 5 and every retained-verifier premise below.

1. Is there exactly one `DeploymentExecutionImage` and cache slot per exact
   deployment build, with no entry/root dimension or route input to the atomic
   constructor?
2. Does K0B gate the union of every canonical public root, reject unsupported
   facts in any such root and exclude only raw private functions unreachable
   from the entire union?
3. Is an HTTP gateway one ordered `Guard -> Pre -> Handler` root, with Phase 1
   guard/pre declarations rejected, no standalone role mint and WebSocket
   `CloseHandler` kept separate and disabled?
4. Does exact operation selection require typed `ContractOperationId` after
   image load, delete `.next()`, and leave cache identity unchanged?
5. Is the post-link verifier independent but non-authoritative, with internal
   `ExecutableFacts` sole-producing the statement schedule and no source,
   registry, entry or effect reconstruction?
6. Can every failure/panic/cancellation/owner-mismatch interleaving publish only
   one complete deployment image or none, never a root subset, heap subset or
   verifier output?
7. Do host, request, VM, scheduler, package-test and test-runner consume the
   same final authority, with test-runner using the production full-deployment
   constructor and all old public pairing/construction paths removed?

There is no unresolved user-visible Phase 1 choice. Operation routes without a
typed `ContractOperationId` fail closed; the accepted HTTP VCP remains a
handler-only gateway. Enabling HTTP guard/pre, WebSocket close or another root
class requires its later phase decision and does not alter deployment cache
identity.

## Amendment r2 (2026-08-14): retire the independent production verifier

This amendment records the architecture ruling made during Phase 5. It
supersedes every earlier DEC1 statement that retains an independent post-link
verifier, the `skiff-runtime-bytecode-verifier` crate, `ExecutableFacts`,
`verify_executable_facts`, verifier-owned `Verified*` execution tables, or a
`link -> verify` production boundary. The preceding text remains as the
historical decision record; none of those superseded names or boundaries is a
compatibility commitment.

The canonical long-term boundary is
[`doc/architecture/bytecode-vm.md` §2.4 and §4](../../../architecture/bytecode-vm.md#24-responsibility-split);
this amendment and the active Phase Contract are its implementation decisions,
not a competing architecture source.

### Authority split

The compiler is the sole source-semantics authority. It decides and emits the
typed facts for effects, placements, value lifecycle and transfer, callable
roles, source/statement attribution, and every other language-semantic claim.
Fingerprint-pinned registries remain canonical data inputs named by those
facts; they do not authorize the linker to rediscover a missing compiler fact.

The linker owns one atomic construction of `DeploymentExecutionImage`. It may
parse compiler-emitted facts, resolve their exact indexes and identities
against the linked package closure and pinned registry rows, and reject an
absent, malformed, dangling, ambiguous, or contradictory reference. It must
not infer or replace source semantics from opcode families, names, namespaces,
contexts, type/shape resemblance, registry membership, or runtime behavior.
In particular, a linker check that an emitted binding resolves to the exact
pinned row is referential consistency, not a second host-effect admission
policy.

There is no separately callable production verification stage. Necessary
bounded structural work may exist only as private construction-local steps of
the atomic image constructor:

- decode and table/index bounds;
- instruction, branch and CFG consistency;
- stack height, underflow, merge and maximum-depth consistency;
- frame/slot bounds and move/liveness consistency;
- call target/index/arity/signature joins;
- suspend/resume site and target correspondence; and
- statement-schedule assembly by mapping exact compiler-emitted statement and
  source facts plus their pinned charging facts into image-local indexes.

That list is a ceiling, not a mandate to recreate the deleted verifier inside
the linker. Missing source-semantic evidence is a compiler/artifact error and
must fail closed; it is never reconstructed by structural analysis. Private
scratch structures cannot escape the constructor, be cached, be paired with
an image, or become an alternate execution input. Only the complete immutable
`DeploymentExecutionImage` may be published.

### Hard cut and failure envelope

The repository removes `runtime/bytecode-verifier` as a production crate and
workspace member. `ExecutableFacts` and `verify_executable_facts` are deleted.
Required limits and runtime-facing structural views move to linker/image-owned
types; consumers use narrow `DeploymentExecutionImage` accessors. This is a
hard cut: no deprecated alias, forwarding crate, feature-selected old path,
test-only legacy constructor, dual dependency, or old Gate selector remains.
Verifier tests are classified before deletion: structural corruption cases
move to linker image-construction tests, while source-semantic test intent
belongs to compiler/artifact tests and must not be transplanted as linker
inference.

A damaged artifact may be rejected while constructing the image. A corruption
that is intentionally handled later must encounter checked image/runtime
lookups and produce a bounded request failure with normal root/resource
cleanup. Neither case may cause undefined behavior, an out-of-bounds access,
a process crash or abort, a partially published image, or a pending/resource
leak. Panics caught at an existing deployment-attempt boundary remain failed
construction; panicking on artifact-controlled data is not an accepted
runtime failure strategy.

Historical receipts and tests that mention the removed stage remain historical
evidence only. The active Phase 5 Contract and MAP own the exact cutover write
set, stage-sentinel renaming, dependency removal, test migration, and Gate
selector replacement. No independent architecture/decision review is required
before implementation. Focused tests, the canonical Gate, frozen-candidate
semantic review and Acceptance must prove one compiler semantic authority, one
atomic linker image mint, no verifier-shaped compatibility surface, and the
complete safe-failure envelope before Phase 5 is accepted.
