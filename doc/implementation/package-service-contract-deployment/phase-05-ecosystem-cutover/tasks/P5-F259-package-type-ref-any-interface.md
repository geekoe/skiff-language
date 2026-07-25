# P5-F259 PackageTypeRef representation for `any Interface`

## Context

The Agent Package cannot publish because inline existential interface types
embed a dependency nominal but have no exact `PackageTypeRef` representation.
First production sites include:

- `packages/agent/tools.skiff:166,171,350,364`;
- `packages/agent/runner.skiff:1211`.

The AST is `TypeExpr::AnyInterface` around `llmApi.LlmClient` (sometimes
nullable). Internal `TypeRefIr` retains the dependency `PackageSymbol`, but
contract type resolution currently tries to encode every `AnyInterface` as
opaque `PackageTypeRef::Local`. It correctly rejects that when the interface is
Package-owned.

`PackageTypeRef` currently has Local, PackageSchema, Container and Nullable but
no existential/interface form.

## Required implementation

- Add an explicit `PackageTypeRef` representation for `any Interface`.
- Its interface target must recursively preserve exact Package/local identity;
  `any llmApi.LlmClient` contains the exact llmApi PackageSchema reference.
- Nullable and container wrapping remain structural around the existential.
- Carry the new form through serialization, hashing, ServiceContract
  validation, assignability, diagnostics, lowering/linking and Runtime boundary
  type resolution.
- Preserve existential semantics: a concrete value must implement the exact
  interface; unrelated interfaces and display-name matches are rejected.
- Do not encode it as opaque Local or erase the embedded Package nominal.

## Acceptance

- Positive fixtures cover local and Package-owned interfaces, nullable and
  nested container forms, and concrete implementors.
- Negative fixtures cover wrong owner/type ID, non-implementor, malformed wire
  form and tampered schema closure.
- Existing artifact identity/golden fixtures are intentionally updated for the
  new canonical wire form.
- Agent PackageArtifact, tests and downstream Agine graph cross all cited
  inline-any errors.
- Relevant compiler/artifact/linker/Runtime tests, workspace check, result and
  commit.
- No compatibility shim is required; the language is unpublished.
- No push, stable operation or disk cleanup.
