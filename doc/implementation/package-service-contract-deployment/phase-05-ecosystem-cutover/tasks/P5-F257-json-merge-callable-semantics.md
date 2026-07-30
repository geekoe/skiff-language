# P5-F257 std.json.merge callable semantics

## Context

F253 proved cross-file Package effect closure is already complete. AIHub HTTP
and WebSocket operations become Unknown through:

```text
applyProviderOptions -> std.json.merge
```

The native signature and Runtime implementation exist, but callable semantics
are absent. Runtime creates a detached JSON result: object/object performs a
shallow cloned overlay; null overlay clones base; other overlays are cloned as
the result.

## Required implementation

- Register only exact `std.json.merge(Json, Json) -> Json`.
- Successful result provenance is Fresh/detached.
- No caller write, alias, escape, same-heap requirement, unknown target or
  suspension.
- Validate exact binding, arity, parameter and return types.
- Reject aliases/lookalikes and malformed signatures.
- Preserve Runtime shallow-overlay/non-object behavior and ensure returned
  nested JSON is detached from both inputs.

## Acceptance

- Artifact/compiler/Runtime tests cover exact semantics, malformed signatures,
  object overlay, null/non-object overlay and detached nested identity.
- AIHub `applyProviderOptions` and explicit ingress operations clear this
  Unknown target.
- Relevant tests, workspace check, diff check, result and commit.
- No push, stable operation or disk cleanup.
