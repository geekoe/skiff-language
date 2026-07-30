# P5-F213 JsonObject.get callable semantics

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

Relay `v1Proxy` first becomes conservative through the dependency chain:

```text
chatgptPlan.responses
  -> transport.responses
  -> rawRequest
  -> currentCredential
  -> refreshCredential
  -> tokenClaims
  -> claimsFromJwt
  -> jsonField
  -> receiver:JsonObject.get@1
```

The lowering target is canonical. The builtin receiver semantics registry
contains `JsonObject.has` and `JsonObject.set`, but not `JsonObject.get`.
Fallback effects then pollute the whole dependency callable.

Unlike `has`, `get` returns a Json value reachable from the receiver. Its
provenance and heap requirements must match the actual Runtime representation;
it must not be guessed as a detached scalar.

## Required implementation

1. Inspect the canonical Runtime handler/value representation for
   `JsonObject.get`.
2. Add exact receiver callable semantics matching whether the returned value is
   a receiver alias, a materialized detached copy, or a precisely modeled
   optional/union of those outcomes.
3. Validate canonical receiver identity, arity, key type, and return type
   against the existing operation/signature.
4. Set write, escape, unknown-target, same-heap, suspension, and return
   provenance flags exactly. Do not conservatively add unrelated effects.
5. Preserve fail-closed behavior for malformed signatures, wrong receiver,
   wrong key/return type, and non-canonical lookalikes.
6. Do not add package or Relay-specific exceptions.

## Acceptance

- Runtime-backed tests prove the actual alias/copy behavior of returned nested
  Json values.
- Positive and negative receiver-semantics tests match that behavior exactly.
- Tests cover missing keys, scalar values, and nested object/array values.
- A focused tokenClaims/jsonField caller shape no longer reports an unknown
  target; any real alias/same-heap requirement remains visible rather than
  being erased.
- Real `chatgptPlan.responses` and Relay `v1Proxy` proceed to Available or
  record the exact next independent blocker.
- Existing compiler/Runtime tests, `cargo check --workspace`, and
  `git diff --check` pass.
- Add `P5-F213-json-object-get-callable-semantics-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

Use this task as immediate authority. Runtime value behavior and the canonical
receiver signature are authoritative for provenance. Ask the primary agent if
the current Runtime behavior cannot be represented by existing callable
effects without changing the public value model.
