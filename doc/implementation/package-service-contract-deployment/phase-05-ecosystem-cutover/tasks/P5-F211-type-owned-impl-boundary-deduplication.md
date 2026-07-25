# P5-F211 Type-owned impl boundary deduplication

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

The fresh Relay contract contains 19 public boundary paths while the canonical
source/API expectation contains 17. The exact extra paths are:

```text
CodexRelayProxy.responsesCompleted
CodexRelayProxy.responsesCompletedResult
```

The same operations are intentionally exposed through the published
`relayProxy` value. Projection additionally treats the nominal type-owned impl
methods as independent service entrypoints, creating duplicate public
operations.

A Package nominal type's impl method is not automatically a service boundary.
Service operations must come from the explicit published API/value surface.

## Required implementation

1. Stop projecting type-owned impl methods as independent service operations
   merely because the nominal type is public.
2. Preserve methods explicitly reachable through a published value/interface
   such as `relayProxy`.
3. Preserve explicitly published free functions and intended service
   entrypoints.
4. Do not deduplicate only by display name; use the source declaration and
   publication ownership model so distinct intentional operations remain
   distinct.
5. Fail closed on ambiguous or multiply-owned publication facts rather than
   silently choosing one.

## Acceptance

- Projection tests cover a public nominal type with impl methods but no
  published service value: no standalone service operations are emitted.
- Tests cover the same methods exposed through one published value: exactly one
  operation per published method is emitted.
- Tests retain distinct intentionally published operations with similar names.
- Fresh Relay contract returns to the expected 17 paths, with both
  `CodexRelayProxy.*` duplicate paths absent and the intended `relayProxy`
  operations present.
- Existing projection/contract tests, `cargo check --workspace`, and
  `git diff --check` pass.
- Add `P5-F211-type-owned-impl-boundary-deduplication-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

Use this task as immediate authority. Follow package publication facts and
service boundary projection only. Ask the primary agent if current API
authoring explicitly declares both the nominal impl and published value as
separate boundaries.
