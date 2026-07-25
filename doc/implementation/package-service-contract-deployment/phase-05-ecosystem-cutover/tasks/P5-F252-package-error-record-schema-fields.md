# P5-F252 Package error record schema fields

## Context

Current clean Relay source accesses `chatgptPlan.OauthError.code` and
`.message`, and the llm-providers source/API declares those fields. Fresh or
retained exact graphs can nevertheless expose an `OauthError` Package schema
without them, blocking Relay source compilation before F245 can execute.

Earlier runs crossed this site only with a different source/artifact state, so
that receipt is not a valid baseline.

## Required investigation and implementation

- Record the exact llm-providers source commit, public API declaration,
  PackageArtifact identity, OauthError stable key/type ID and canonical
  descriptor fields on both a passing and failing graph.
- Trace declaration -> schema closure -> artifact store -> dependency source
  projection for Package-owned error records.
- Preserve all public fields and their exact nominal references.
- Ensure errors are not treated as fieldless marker types merely because they
  implement `ErrorPayload`.
- Reject stale/tampered schema identities rather than silently projecting a
  different shape.

## Acceptance

- Focused ordinary record and `ErrorPayload` record schema-field tests,
  including cross-Package field access.
- Exact artifact round-trip/tamper tests.
- Fresh std -> llm-api -> llm-providers -> Relay graph crosses
  `OauthError.code/message` without source workarounds.
- Re-run F245 Relay 23-case suite with F249 and record its result.
- Relevant compiler/deployment tests, workspace check, result and commit.
- No push, stable operation or disk cleanup.
