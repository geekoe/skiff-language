# P5-F221 Public LocalType boundary normalization

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

`llm-api/api.yml` explicitly publishes `ResponsesMaterializationResult` and two
functions using it. The source type is a public nominal union with four string
literal discriminator branches.

The callable ABI still carries the type as `TypeRefIr::LocalType { typeIndex:
2 }`. Boundary projection only normalizes `ServiceSymbol`; it does not use the
owner module plus LocalType index to recover the already-generated public
Package schema reference. It therefore expands the type structurally and fails
on string literals as `UnsupportedBoundaryType`.

Package schema projection already has the canonical `(module, symbol) ->
PackageSchema` mapping. Do not relax literal boundary rules.

## Required implementation

1. Normalize an owner-package LocalType used in a callable ABI to its canonical
   PackageSchema reference when, and only when, the source type is explicitly
   public in api.yml.
2. Use declaration identity/type index mapping, not display-name guessing.
3. Apply to parameters, returns, nested containers, and reachable callback
   signatures.
4. Preserve rejection of unpublished LocalType, missing schema records,
   dependency ownership mismatch, unsupported callback/function closure, and
   ambiguous mappings.
5. Do not structurally inline the nominal type or add string-literal boundary
   exceptions.

## Acceptance

- Public nominal literal-union LocalType parameters/returns project as
  PackageSchema and are boundary-type Available.
- Unpublished LocalType and missing/forged schema mappings remain rejected.
- Nested/container/callback positive and negative coverage passes.
- Real llm-api completed functions no longer have
  `UnsupportedBoundaryType` from ResponsesMaterializationResult.
- Existing projection/contract tests, workspace check, and diff check pass.
- Add `P5-F221-public-local-type-boundary-normalization-result.md` and commit.
- Do not push or operate stable.
