# P5-F253 Cross-file publication effect closure

## Context

F251 explicit AIHub HTTP/WebSocket operations compile, but their ServiceContract
facts are conservatively Unavailable:

```text
provenance: unknown(unknownCallTarget)
resolvedCallTargets: {}
returnsCallerAlias: true
```

The entrypoints and same-file calls lower as `localExecutable` and are followed.
Calls to another source file in the same Package lower as
`publicationExecutable`; callable-effect analysis does not resolve those
targets into the Package's FileIR closure.

Affected chains include HTTP/WS -> provider catalog and managed provider
transport. The alias result is only conservative Unknown expansion, not an
observed caller alias.

## Required implementation

- Resolve exact same-Package `publicationExecutable` targets across all FileIR
  units in the linked Package.
- Include their callable facts in fixed-point effect/provenance analysis.
- Preserve recursion handling and deterministic code/publication identity.
- Distinguish same-Package publication targets from Package dependency and
  ServiceContract calls.
- Missing, ambiguous or mismatched publication targets remain fail-closed.
- Do not add AIHub wrappers or mark publication calls pure by default.

## Acceptance

- Multi-file Package fixtures cover direct, transitive and recursive
  publication calls.
- Effects, return/throw provenance, writes, escapes and suspension propagate
  across files.
- Missing/ambiguous target negatives remain Unknown/Unavailable.
- AIHub explicit HTTP and WebSocket operations become Available or expose the
  next exact real effect blocker.
- Relevant compiler/projection tests, workspace check, result and commit.
- No push, stable operation or disk cleanup.
