# P5-F259 Result: PackageTypeRef representation for `any Interface`

## Outcome

Implemented an explicit existential interface representation in both
`PackageTypeRef` and `ContractTypeRef`:

```text
AnyInterface {
  interface,
  arguments
}
```

The target and generic arguments recursively retain exact local or
PackageSchema identity. Nullable and container wrappers remain structural
outside the existential.

## Completed paths

- strict serde wire form and identity hashing;
- PackageArtifact callable and constant validation;
- ServiceContract normalization, schema-closure collection and validation;
- source resolution, substitutions, exact assignability and diagnostics;
- PackageArtifact boundary projection to ServiceContract;
- File IR lowering with exact interface target and canonical type arguments;
- Runtime loader closure collection and boundary type-plan resolution;
- ordinary wire/persistence lanes remain fail closed for existential values.

Exact identity validation rejects malformed wire, opaque interface targets,
undeclared owners, wrong stable keys/type ids, non-interface schema records and
tampered schema closure.

## Verification

- `cargo check --workspace`: passed.
- Focused artifact/compiler/runtime suites passed:
  - artifact-model: 131 tests;
  - artifact-identity: 79 library tests and 8 CLI tests;
  - compiler-lowering: 43 tests;
  - runtime-boundary: 175 tests.
- Compiler projection existential fixture passed.
- Generic `any pkg.Reader<string>` lowering retains the package selector and
  canonical `string` argument.
- Real Agent publication using a fresh copy of the F251 exact store crossed all
  cited inline existential failures in `tools.skiff` and `runner.skiff`.

## Next production gate

Agent publication now reaches an unrelated existing JSON contextual typing
gate in `canonical_execution`: `LlmRequest` is rejected where `std.json.encode`
expects `Json` (at lines 16 and 60). Because no Agent pointer is produced,
downstream Agine publication cannot yet run. This is the exact next task; it is
not an existential representation failure.

The repository-wide compiler selector still reports its existing fixture debt
(state-requirement fixtures, prelude initialization, package-requirement
fixtures and stale golden identities). The F259 focused suites and workspace
check are green.
