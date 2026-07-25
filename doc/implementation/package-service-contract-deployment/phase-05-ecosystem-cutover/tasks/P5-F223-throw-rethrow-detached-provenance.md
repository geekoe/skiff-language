# P5-F223 Throw/rethrow detached provenance

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

After F220, Relay `v1Proxy` retains unknown/write/throw-alias effects through:

```text
archiveSseResponseStream
  -> catch std.file.createFromStream(...)
  -> rethrow exception
```

Compiler callable-effects handling currently treats statement and expression
rethrow as unsupported control flow and unconditionally joins all effects.
Throw operand origins are also propagated as caller throw aliases.

Runtime throw and rethrow serialize through canonical wire values and rebuild
the error payload. Typed catch variables are already modeled Fresh. Therefore
the thrown envelope/payload does not retain request-heap alias identity.

## Required implementation

1. For throw and rethrow, evaluate the operand/expression normally and preserve
   its real evaluation effects and suspension.
2. Record the emitted thrown value as detached Fresh wire provenance rather
   than propagating caller alias/same-heap identity.
3. Apply consistently to statement throw/rethrow and expression forms.
4. Preserve exact typed throw identities and typed catch behavior.
5. Unknown/dynamic/ill-typed exception targets remain fail-closed with precise
   diagnostics; do not convert arbitrary unsupported control flow to safe.
6. Do not erase writes/escapes performed while constructing or evaluating the
   exception operand.

## Acceptance

- Tests cover throwing a caller-derived payload, local Fresh error, typed
  catch/rethrow, nested catch, and operand evaluation effects.
- Known typed rethrow does not add unknown/write/alias/same-heap effects by
  itself.
- Unknown/dynamic exception targets remain rejected or conservative.
- Runtime wire roundtrip tests prove detached error payload identity.
- Real Relay archiveSseResponseStream no longer receives all-effects solely
  from rethrow; `v1Proxy` proceeds or records the next exact blocker.
- Existing source/Runtime tests, workspace check, and diff check pass.
- Add `P5-F223-throw-rethrow-detached-provenance-result.md` and commit.
- Do not push or operate stable.
