# P5-F255 Package test stream output context

## Context

Relay full tests now execute, but four package response-health cases fail with:

```text
emit used outside a stream output context
```

The tested callable is a valid stream producer. Focused production projection
tests pass, so the failure is in package-test assembly invocation/context
construction rather than SSE parsing.

## Required investigation and implementation

- Record the exact four test callables and their declared stream signatures.
- Trace compiler FileIR, linker callable kind and Runtime test invocation
  context.
- Install a stream output collector only for a callable whose exact signature
  produces a stream.
- Preserve ordinary callable behavior and nested stream forwarding.
- Reject `emit` in genuinely non-stream callables.

## Acceptance

- Package-test fixtures cover direct emit, nested producer, forwarded stream
  and illegal ordinary emit.
- The four Relay response-health cases execute without this context error.
- Runtime/test-runner tests, workspace check, result and commit.
- No push, stable operation or disk cleanup.
