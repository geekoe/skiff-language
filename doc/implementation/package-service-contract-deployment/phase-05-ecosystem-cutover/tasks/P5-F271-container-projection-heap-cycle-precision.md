# P5-F271 Container projection heap-cycle precision

## Authority

- `doc/reference/static-semantics.md`
- `doc/reference/runtime.md`

## Dependencies

- P5-F232 formal-indexed write/escape transfer.
- P5-F258 conditional heap transfer.

## Problem

Callable-summary replay currently collapses a value projected from a container
element or nested field back to the container formal root. The cycle guard then
rejects ordinary writes as `unsupportedHeapStore`.

Confirmed production shapes:

- `llm-api responses.materializeCompletedResult`: a state obtained through
  `Map.get` is updated and written back with `Map.set`.
- Relay `proxy_runtime.handleUpstreamStream`: a fresh state object is passed to
  `streamUpstreamUnsafe`, whose parameter stores are replayed onto that state.
- AIHub tool projection: a fresh `JsonObject` contains a field read from a
  caller-owned array element, is then mutated locally and pushed into a fresh
  output array. The fresh container root must remain distinct from its
  caller-reachable payload.

## Required implementation

- Preserve distinct, stable provenance for container-element and nested-field
  projections through callable summaries, argument/formal substitution and
  fixed-point joins.
- Model a fresh container root separately from the values reachable through
  its payload. Mutating the fresh root must not become a caller write merely
  because one contained field aliases caller data.
- Reject real self-cycles and caller-owned cycles exactly as before.
- Do not whitelist Relay, `Map.set`, source positions or callable names.
- Keep summary serialization and artifact identity deterministic.

## Acceptance

- Focused positive regressions model all confirmed production shapes.
- A focused AIHub-shaped regression covers fresh-container root/payload
  separation while retaining caller reachability of the contained field.
- Negative regressions still reject direct and transitive real heap cycles.
- Fresh Relay and llm-api artifacts no longer report
  `unsupportedHeapStore` for the confirmed call sites.
- Compiler-source tests, workspace check, result and commit.
