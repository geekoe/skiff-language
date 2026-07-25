# P5-F239 Package nominal alias target typing

## Context

P5-F236 preserves exact cross-Package nominal identities. Real AIHub now
advances past all same-name std and llmApi identity errors, but literals and
expanded alias unions are not accepted where an exact Package-owned nominal
alias is expected. The first affected types include `LlmRole`,
`LlmReasoningLevel` and `LlmApiFormat`. Later diagnostics mention untyped object
literals and iterable expressions.

## Required investigation and implementation

- Reproduce the first real AIHub error and record the source expression,
  expected exact `PackageTypeRef`, inferred representation and target-typing
  path.
- Define target typing for values of Package-owned aliases without erasing the
  nominal identity:
  - a compatible literal or union representation may construct the expected
    exact nominal alias;
  - an already nominal value must retain and compare its exact owner/key/type
    identity;
  - incompatible literals and a nominal from another owner/type remain
    rejected.
- Apply the same rule consistently to arguments, assignments, returns, record
  fields, object literals and iterable element inference where the expected
  type is available.
- Do not add AIHub-specific coercions, compare display names, structurally
  equate unrelated nominals, or guess artifact versions.

## Acceptance

- Focused positive and negative tests cover literal-union aliases, object
  representations, iterable elements and exact cross-Package identity.
- Real AIHub publishes against the exact Relay contract, or reaches a next
  independent blocker whose full types and source location are recorded.
- Re-run relevant compiler suites, workspace check and diff check.
- Result document and commit.
- No push, stable-instance operation, source workaround or disk cleanup.
