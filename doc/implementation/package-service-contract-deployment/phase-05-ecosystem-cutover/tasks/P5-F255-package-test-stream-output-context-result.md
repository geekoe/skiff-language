# P5-F255 Package test stream output context result

## Outcome

- Runtime stream-producer recognition now accepts a linker-resolved
  `LinkedCallTarget::PackageDirect` and uses its exact executable address.
- The existing producer gate remains unchanged after target resolution: the exact
  callee must both contain `emit` and declare `Stream<T>`. Only then is a channel,
  stream sink and exact item type plan installed.
- Ordinary package callables still execute through the ordinary package-direct
  lane. An `emit` in a callable without a `Stream<T>` signature is not granted a
  sink and therefore continues to fail closed.
- Nested producer preparation and stream forwarding continue through the shared
  `StreamProducerExecution` path; no package-test-only stream bridge or response
  artifact was introduced.

## Relay evidence

All four failing callables are ordinary generated package-test entrypoints with
the declared signature `() -> void`:

- `package safe response preserves quota reset metadata for relay health`
- `package safe response keeps retry classification without leaking raw error`
- `package safe response keeps 401 and 403 fatal classifications`
- `package safe 5xx response retains unknown recovery failover state`

Each enters the same exact stream chain:

- local `applyPackageResponse(...) -> string`;
- package-direct
  `llmProviders/testing.chatgptPlanSafeResponses(integer,
  Array<std.http.HttpHeader>, string) ->
  Stream<std.http.HttpResponseStreamEvent>`;
- its nested package producer `safeResponses(...) ->
  Stream<std.http.HttpResponseStreamEvent>`;
- Relay forwarding producer
  `handlePackageResponse(Stream<std.http.HttpResponseStreamEvent>, string?,
  UpstreamSourceSelection) -> Stream<std.http.HttpResponseStreamEvent>`.

The file-scoped isolated package test passed all four cases. The complete Relay
run reached 95 cases: 75 passed and 20 independently failing cases remained in
state-order assertions and HTTP test-double request projection. None reported
`emit used outside a stream output context`, and all four response-health cases
remained green.

## Coverage

- Added a real linked/admitted assembly fixture whose consumer iterates a generic
  `PackageDirect` producer emitting `true` then `false`.
- The fixture validates exact generic item substitution, direct emit, nested
  producer execution and forwarding through the ordinary test consumer.
- Existing source/compiler emit validation and runtime no-sink behavior retain
  the illegal ordinary-emit rejection.

## Validation

- `cargo test -p skiff-runtime-host
  typed_execution_package_direct_stream_installs_exact_producer_context_full_chain
  --no-fail-fast`: 1 passed.
- `cargo test --no-fail-fast -p skiff-runtime-eval -p skiff-runtime-host`:
  runtime-eval 132/132, runtime-host 261/261, active assembly 2/2.
- Relay `package_response_health.test.skiff`: 4/4 passed.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.

The canonical runtime selector remains blocked by two pre-existing artifact
boundary findings in `runtime/host/src/loader/active_assembly_context.rs`.
The test-runner selector completed 44 passing tests, with three pre-existing
failures: one timing-sensitive HTTP fixture and two stale package deployment
expectations. These files are outside F255 and were not changed.

No push, stable-instance operation or disk cleanup was performed.
