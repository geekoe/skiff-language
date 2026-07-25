# P5-F271 Container projection heap-cycle precision result

## Result

- Callable analysis now keeps a compiler-internal
  `CallerParameterProjection` distinct from the caller parameter root.
- Field access, `Map.get` and `JsonObject.get` preserve that projected
  provenance through helper summary replay.
- Artifact-facing summaries remain conservative: the internal projection is
  folded back to `CallerParameter` at the publication boundary, so no new
  cross-Package trust claim or wire form was introduced.
- A value obtained from a container can be mutated and written back without
  being mistaken for the container itself.
- A fresh state passed through a helper parameter-store remains distinct from
  a real root self-cycle.
- Direct and transitive real heap cycles continue to fail closed as
  `UnsupportedHeapStore`.

Fresh Internals projection confirmed:

- `llm-api responses.materializeCompletedResult`: analyzed and available.
- Relay `v1Proxy`: available.
- Relay completed-response operations: available.

## Validation

- `cargo test -p skiff-compiler-source --lib`: 286 passed.
- Focused container projection and helper parameter-store regressions passed.
- `cargo check --workspace`, formatting and diff checks passed.
