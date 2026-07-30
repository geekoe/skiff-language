# P5-F236 Cross-package nominal type identity

## Context

P5-F234 makes the exact Package schemas required by a ServiceContract
available before source compilation. Real AIHub now crosses the former
`MissingPackageSchema` and service-alias import failures, but expression type
checking reports comparisons such as:

```text
expected Array<std.http.HttpHeader>, found Array<std.http.HttpHeader>
```

The display names are identical, so the diagnostic currently hides the
identity component that differs. Other errors in the same compile mention
llmApi union facts.

## Required investigation and implementation

- Reproduce the real Relay -> AIHub compile with one exact canonical artifact
  graph.
- At the first mismatch, record both complete internal type identities:
  owner, package version/build/local ABI, schema type ID, generic arguments and
  the source expression.
- Determine whether one side came from a ServiceContract schema, a direct
  Package dependency schema, compiler-owned std, or a stale/local source type.
- Fix the shared canonical type-resolution or equality path. Do not compare
  display strings, erase nominal identity, guess a latest artifact, or add an
  AIHub-specific conversion.
- Make diagnostics distinguish types whose display names are equal but
  canonical identities differ.
- Audit the llmApi union errors after the first identity defect is fixed and
  prove whether they share the same cause.

## Acceptance

- Focused positive test: the same exact Package type reached through a direct
  Package dependency and a ServiceContract is accepted as one nominal type.
- Negative tests: different build, local ABI, owner, or schema type ID remain
  incompatible even when their display names match.
- Real AIHub publishes against the exact freshly published Relay contract, or
  the next independent blocker is identified with full internal identities.
- Relevant compiler tests, workspace check and diff check pass.
- Result document and commit.
- No push, stable-instance operation, source workaround, or disk cleanup.
