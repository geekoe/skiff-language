# P5-F230 Fresh-local heap store analysis

## Decision

The language/compiler will prove controlled mutation of Fresh local records.
Relay will not be rewritten into explicit immutable state threading merely to
work around missing analysis.

## Context

Relay creates a Fresh `UpstreamStreamState`, passes it through known helpers,
mutates fields across loops and stream suspension, and never stores the state
in a Map, Array, database, unknown call, return, throw, or external sink.

Compiler callable-effects currently accepts only a narrow direct-scalar
parameter-field write. Other member assignment calls
`mark_unsupported_heap_store()`, which joins all effects and reports
`UnsupportedControlFlow`.

Known helper calls already map callee write effects through actual argument
provenance: a write to a formal maps to caller-write only when the actual value
is caller-reachable. The missing piece is precise member-store provenance and
Fresh-root ownership/taint.

## Required analysis

1. Track stable Fresh local root identity through direct local aliases and
   known helper formal/actual mapping.
2. For `base.field = rhs`, evaluate both base and rhs:
   - a proven Fresh local root may be mutated without outward write effects;
   - a caller-parameter root records caller-reachable write and same-heap
     requirements;
   - unknown/native/external ownership remains fail-closed.
3. A Fresh aggregate that stores caller-reachable or otherwise aliased `rhs`
   must become transitively tainted. Returning, throwing, escaping, or storing
   that aggregate later must expose the contained provenance; Fresh must not
   hide a caller handle.
4. Preserve root/taint identity through known helper calls and fixed-point loop
   analysis. Suspension alone does not revoke ownership.
5. Invalidate/fail closed when a tracked root or alias enters:
   - Map/Array/container storage not modeled precisely;
   - database state;
   - unknown/dynamic/external callees;
   - returned/thrown/native escape paths;
   - ambiguous alias merges or unsupported destructuring.
6. Do not weaken boundary eligibility or convert arbitrary heap stores to safe.
7. Replace the misleading `UnsupportedControlFlow` reason for rejected heap
   stores with an accurate heap-store/ownership diagnostic.

## Acceptance

- Positive tests:
  - Fresh record field writes, including reference/container-valued fields;
  - local alias then field write;
  - known helper mutating a Fresh actual;
  - loop and real suspension between controlled writes;
  - Relay-shaped 24-field state passed through multiple helpers.
- Context tests:
  - the same helper called with a caller-owned record reports write/same-heap;
  - storing caller-derived content taints a Fresh root and returning it is
    rejected;
  - detached/native-Fresh content does not create caller alias.
- Negative tests:
  - Map/Array/DB storage, unknown call, external escape, return/throw, ambiguous
    aliases, and branch merges remain conservative.
- Real Relay `v1Proxy` loses `UnsupportedControlFlow` without losing legitimate
  stream escape/suspension effects.
- Existing compiler tests, workspace check, diff check, result document, and
  commit.
- Do not push or operate stable.
