# P5-F445H-I7-S1 Host/runtime/Router cross-layer receipt

## 1. Parent chain and DAG position

Direct parents:

- `P5-F445H-I7R-cross-boundary-readiness-preflight-result.md`, section
  `S1 — Host/runtime/Router cross-layer receipt`;
- `P5-F445H-I7-S0-real-source-artifact-checkpoint-result.md`, which records
  `S0_COMPLETE = YES` and freezes the checked-in current-scope fixture identity tuple;
- final I6 acceptance:
  `P5-F445H-I6K-R4-independent-current-scope-reacceptance-result.md`.

The I7R result traces through its task and Phase 05 DAG to the sole architecture source
`doc/architecture/package-service-contract-deployment.md`. This leaf only completes the
test-owned S1 receipt; it does not redefine runtime, Host, Router, protocol, timeout, cancellation,
service-call or authoring semantics.

```text
S0 exact source/artifact checkpoint + final I6 acceptance
  -> S1 exact artifact consumption by Host and Router
  -> J hermetic join dependency released
```

`P0_COMPLETE = YES` and `T0_COMPLETE = YES` are recorded sibling prerequisites for J, but are not
inputs to this Skiff-only implementation.

## 2. Frozen inputs and execution identity

| Item | Value |
| --- | --- |
| Skiff baseline commit | `6b5b71014800e4b18bc8ec70400510185e856fd6` |
| Skiff baseline tree | `dc6b9d5a2438d885770e074243368acde54cbcca` |
| integration branch | `codex/package-service-phase-05` |
| leaf branch | `codex/p5-f445h-i7-s1-host-router` |
| leaf worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-s1-host-router` |
| integration owner | `/root/phase05_integration_steward` |
| S0 assembly | `skiff-runtime-assembly-v2:sha256:ec66d8a209e65198ee5b82086a365a4b3a98021ef8117e2572c66fee8eac5f6e` |
| official package candidate | `b06d7aaf16b6914837de1f74920fd3f626040472` (provenance only; not consumed by S1) |

Evidence remains valid only while the S0 fixture/source identities, canonical std/compiler
producer, Host admission/execution path, Router artifact reader/transport path and F442 corpus
remain unchanged.

## 3. Read-only preflight result and real path

The baseline has one canonical multi-root producer,
`skiff_test_runner::package_service_host_fixture::prepare_package_service_host_fixture`. S0 uses
it to compile the checked-in
`test-runner/fixtures/package-service-current-scope/{helper,provider,consumer}` roots and publish
their exact package, contract, deployment and RuntimeAssembly records. The existing
`skiff-package-service-smoke-fixture --prepare-host-base` binary exposes the same producer to
Node tests after the canonical std bootstrap.

Current Host HTTP integration tests instead call `write_service_fixture` to generate unrelated
Rust-owned source and rewrite artifact records for their legacy schema-neutral fixture. Current
Router compatibility tests compile `compiler/tests/fixtures/router-websocket-fixture`; stream and
unary dispatch tests use hand-written identities/snapshots. Those paths cannot prove consumption
of the S0 artifact.

S1 will:

1. generate an isolated artifact root from the exact checked-in S0 fixture through the canonical
   producer;
2. load/admit the exact RuntimeAssembly and referenced records through the filesystem resolver in
   Host tests, without synthetic source, record rewriting or identity substitution;
3. load/join the same exact records through Router's filesystem snapshot reader and use the exact
   HTTP unary/server-stream ingress bindings in observable Router dispatch tests;
4. retain I6's focused carrier/deadline/late-settlement selectors and F442 verifier as the
   already-frozen runtime/wire portions of the receipt.

This is a hermetic non-live receipt: all artifacts and work roots are temporary and owner-cleaned.
It does not start stable, use MongoDB, reload artifacts, access external network, OAuth, browser or
live targets.

## 4. Write ownership

Expected implementation write set:

- `runtime/host/src/host/router_session/tests/runtime_assembly_request.rs`;
- `runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs`;
- one narrowly scoped module below
  `runtime/host/src/host/router_session/tests/runtime_assembly_request/**` if separation keeps the
  existing test file cohesive;
- `router/tests/helpers/compilerArtifacts.ts`;
- `router/tests/compilerGeneratedManifestCompatibility.test.ts`;
- `router/tests/assembly-http-gateway-stream.test.ts`;
- `router/tests/runtime-assembly-unary-dispatch.test.ts`;
- F442 verifier/corpus only if the exact current identity tuple requires a mechanical refresh;
- this task and its result document.

Mechanical module declarations, helper types and test fixtures on the same call chain may be
included and must be listed in the result. No active sibling Skiff task owns these files at the
frozen baseline.

Forbidden:

- runtime, Host, Router, compiler, deployment or artifact production;
- protocol/schema/identity constants or public API/ABI/wire behavior;
- S0 source semantics or exact artifact identity tuple;
- Internals, `skiff-packages`, shared stable configuration, live scripts or external state;
- receive/peer-request/public cancellation compatibility, `-32800`, legacy service relay, fallback
  readers or synthetic v2 deployment acceptance.

If exact S0 artifacts expose a production defect, this leaf stops with a repo-local blocker and
does not repair production.

## 5. Completion matrix

Positive:

- canonical producer emits the exact S0 File IR/package/contract/deployment/gateway/assembly
  identity tuple from tracked source;
- Host filesystem resolution admits the exact assembly and exposes exact HTTP unary,
  server-stream and connect routes without record rewriting;
- Router filesystem loading joins the exact consumer and provider closure;
- exact unary and server-stream bindings reach Router's observable response path with status,
  headers, ordered chunks and one terminal end;
- tracked source retains nested timeout around HTTP unary/stream, WebSocket outbound request,
  file, Actor and first canonical service call; the WebSocket call remains three business
  parameters with transport correlation hidden;
- I6 focused selectors continue to prove current-scope deadline/tie-break/late-settlement ownership,
  and first service call does not consume deployment timeout.

Negative:

- wrong assembly/gateway identity or generation fails closed;
- old identity generations and synthetic deployment-v2 input are not accepted by the exact
  filesystem readers;
- receive branch, `peerRequestId`, public peer cancellation/`-32800` and legacy service relay are
  absent from the S1 positive path;
- late response/duplicate terminal cannot create a second observable settlement.

`S1_COMPLETE = YES` requires all scoped positive/negative checks to pass on the implementation
tree, a clean actual write set, and a separate result commit. It releases only the S1 parent of J;
it does not complete J, L0, L1 or I7.

## 6. Focused evidence owner

S1 owns only non-live focused evidence:

```bash
cargo test -p skiff-runtime-host host_current_scope_compiled_artifact --locked
cargo test -p skiff-runtime-eval f445h_i6_carrier_delivery_receipt --locked
cargo check -p skiff-runtime-host --tests --locked
pnpm --dir router exec vitest run \
  tests/compilerGeneratedManifestCompatibility.test.ts \
  tests/assembly-http-gateway-stream.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts
pnpm --dir router type-check
node cross-system-fixtures/package-service-ecosystem/verify.mjs --self-test
node cross-system-fixtures/package-service-ecosystem/verify.mjs --combined-probe
node cross-system-fixtures/package-service-ecosystem/verify.mjs --runtime-wire-self-test
cargo fmt --all -- --check
git diff --check
```

Selectors must be non-zero. Locked Rust checks, router type-check, formatter and diff hygiene are
required. S1 does not run full `verify`, stable/live/network/Mongo gates or J's final matrix.

## 7. Commit and handoff protocol

The task contract, implementation and result are separate commits. The result records exact
commits/trees, actual write set, command ledger, acceptance matrix, blockers and:

```text
S1_COMPLETE = YES|NO
J_S1_PREREQUISITE_SATISFIED = YES|NO
```

The leaf reports directly to `/root/phase05_integration_steward` and the parent Agent. It does not
merge, delete its first-level worktree/branch, push or mutate the integration worktree.
