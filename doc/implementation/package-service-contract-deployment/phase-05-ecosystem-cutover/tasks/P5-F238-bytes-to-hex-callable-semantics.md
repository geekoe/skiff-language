# P5-F238 bytes.toHex callable semantics

## Context

P5-F237 buffers raw SSE bytes correctly and searches for the byte delimiter
without prematurely decoding incomplete UTF-8. Its canonical compile reaches:

```text
receiver:bytes.toHex@1 [UnknownCallTarget]
```

The native signature and Runtime implementation already exist. Only exact
compiler callable semantics are missing.

## Required implementation

- Register only canonical `bytes.toHex() -> string`.
- The result is a fresh scalar string with no caller write, alias, escape,
  same-heap requirement, unknown target, or suspension.
- Validate receiver, zero arguments, return type, native binding and Runtime
  route.
- Reject malformed signatures and similarly named aliases/lookalikes.
- Do not change UTF-8 decoding rules or generalize unrelated bytes helpers.

## Acceptance

- Artifact/compiler/Runtime positive and negative tests mirror the exact
  `bytes.fromHex` safety matrix where applicable.
- P5-F237 focused tests compile and execute.
- The canonical Relay split-UTF-8 case passes.
- Complete Relay file and service suites run to their next independent
  failures, with exact source locations and targets recorded.
- Workspace check, diff check, result document and commit.
- No push, stable-instance operation, source workaround or disk cleanup.
