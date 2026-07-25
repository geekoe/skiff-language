# P5-F234 Pre-source contract schema resolution

## Context

AIHub consumes Relay as a ServiceContract dependency. Relay's contract
correctly requires five `skiff.run/std` Package schema type IDs and does not
inline schema records. The canonical std PackageArtifact/store contains the
complete verified index and records.

Contract dependency validation runs during source compile and receives an empty
`resolved_package_schemas` owner map. Authoring currently adds exact direct
packages and compiler-owned std only to `available_canonical_packages`.
Existing store-backed schema resolution runs later in the projection pipeline,
after source compile, so it cannot satisfy contract validation.

## Required implementation

1. Before source compile, resolve verified `ResolvedPackageSchema` bundles from
   the canonical store for:
   - exact manifest direct package dependencies;
   - compiler-owned exact std.
2. Pass those bundles into PackageCompileInput contract validation.
3. Use exact artifact/version/build refs and manifest aliases. Do not choose
   latest by package ID or infer undeclared dependencies.
4. A non-std schema owner required by a ServiceContract must be an exact direct
   package dependency of the consumer.
5. Share the same pre-source resolution path with canonical package tests;
   avoid divergent authoring/test-runner implementations.
6. Preserve full index/type-record identity, public-nameability, reachable
   closure, owner, version, and tamper validation.
7. Do not inline schema into ServiceContract or weaken MissingPackageSchema.

## Acceptance

- Consumer declares only Relay service while Relay contract uses std types;
  canonical std pointer exists: source compile succeeds.
- Direct package owner required by contract plus exact consumer dependency:
  succeeds.
- Owner neither direct nor std: MissingPackageSchema.
- Missing/tampered std pointer/index/record and duplicate owner/version/alias:
  fail closed precisely.
- Authoring and package-test paths share the behavior.
- Real AIHub publish succeeds and Agine proceeds to its next gate.
- Existing compiler/test-runner tests, workspace check, diff check, result,
  commit.
- No push or stable operations.
