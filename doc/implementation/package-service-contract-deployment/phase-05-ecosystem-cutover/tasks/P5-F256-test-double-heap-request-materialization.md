# P5-F256 Test-double heap request materialization

## Context

Five Relay unary Responses tests reach the
`std.http.client.sse` test double, but request-subset matching receives an
internal HeapHandle rather than a materialized `HttpRequest`.

Test-double matching is a test boundary and must compare canonical values, not
interpreter heap addresses.

## Required implementation

- Materialize callable receiver/arguments from the active RequestHeap before
  matching test-double request subsets.
- Preserve exact typed records, arrays/maps, bytes and nullable values.
- Match without mutating or consuming the production argument.
- Keep response sequences and typed error doubles unchanged.
- Fail closed on invalid handles or type mismatches; do not stringify handles.

## Acceptance

- Runtime/package-test fixtures cover nested HTTP/SSE request records, bytes,
  nullable fields, invalid handles and nonmatching subsets.
- The five Relay unary Responses tests reach their intended double responses.
- Account multi-request double sequence remains green.
- Relevant Runtime/test-runner tests, workspace check, result and commit.
- No push, stable operation or disk cleanup.
