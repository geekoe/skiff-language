# P5-F68 Artifact-native dependency type facts

- Authority: `doc/architecture/package-service-contract-deployment.md`, independently compiled
  package artifacts and exact dependency identity/type linkage.
- Candidate: current Skiff Phase 5 integration; D61 proved dependency artifacts are loaded but
  `source_compile` passes `package_facts: None`.
- Worktree: create `skiff-p5-f68-dependency-type-facts` from current Skiff integration.
- Write owner: compiler driver canonical dependency loading, compiler source compile/type-resolution
  model, and focused artifact projection regression tests.
- Required outcome: construct dependency type facts only from identity-validated
  `PackageArtifact.package_local_abi.public_symbols` descriptors and feed them into
  `TypeResolutionModel`. Cover records, aliases, literal unions, nested fields and arrays. If
  exported interface method facts require exact FileIR, load only that artifact's verified FileIR;
  never dependency source.
- Fail closed: missing/tampered pointer, artifact, coordinate/version/alias, public path, descriptor,
  ABI/build identity; no ambient source/root fallback. Changing or hiding dependency source after
  publish must not affect consumer compile.
- Validation: focused positive/negative compiler tests plus the smallest `llm-api`→`llm-providers`
  isolated publish/compile probe if practical. Do not edit Internals consumers, stable, merge, push,
  or run the full gate.
- Deliver one commit and evidence; report the next non-checkpoint diagnostics separately.

