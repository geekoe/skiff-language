# P5-F241 Contract stream expression typing result

## Outcome

Package dependency callable analysis now retains the callable's canonical
`PackageCallableSignature`. Expression typing consumes that signature when the
ordinary package source reconstruction cannot name the result, preserving a
`Stream<T>` through a direct call, an inferred or annotated local binding and
`for` iteration. The returned expression fact is projected through the
consumer's validated package schemas, so a dependency-owned `T` keeps its exact
Package nominal identity.

Source-reconstructable package-local result types continue through the existing
owner-aware path. The canonical-signature fallback deliberately excludes
package-local slots and service-local symbols rather than treating their local
indices or names as consumer-owned identities.

Stream functions now distinguish two valid return meanings:

- `return null` (or a valueless return) is the ordinary completion of an
  `emit`-producing body;
- returning another `Stream<T>` forwards a stream expression and is checked
  against the declared exact result.

Other completion values remain rejected. Scalar package calls remain
non-iterable.

## Focused validation

- package stream direct expression, inferred binding, annotated binding and
  iteration: passed;
- scalar package call iteration rejection: passed;
- non-null stream-producer completion rejection: passed;
- `cargo test -p skiff-compiler-source --lib --no-fail-fast`: 278 passed;
- `cargo check --workspace`: passed;
- `git diff --check`: passed.

## Real AIHub validation

The worktree was rebased onto integration `e84f765`, including P5-F240 and
P5-F242, then AIHub was built with the canonical CLI against the retained
isolated store `/tmp/skiff-f227-relay.iB6dOG`. No stable instance was used.

All task-cited stream diagnostics are gone:

- `internal.aihub_service.skiff`: 1550, 1559, 1566, 1569 and 1582;
- `internal.managed_provider_transport.skiff`: 27 and 122.

This includes the `llmProviders.streamChat` package call at 1566, whose exact
result is `Stream<agine.ai/llm-api::LlmStreamEvent>`, and the decoded stream
bindings at 1546, 1578 and 120.

The build now stops only at independent JSON/target-typing diagnostics,
beginning with Package nominal alias values used as JSON fields at
`internal.aihub_service.skiff:1691`, plus the previously recorded untyped
object literal at 1432.

There was no push, stable-instance operation or disk cleanup.
