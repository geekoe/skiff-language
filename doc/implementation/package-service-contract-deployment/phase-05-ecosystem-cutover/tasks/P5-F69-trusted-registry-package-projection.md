# P5-F69 Trusted registry official package and projection

- Authority: `doc/architecture/package-service-contract-deployment.md`, trusted registry package,
  exact native binding requirements, and capability authorization.
- Shared checkpoint: current Skiff integration after F66 compiler-owned registry native authority.
- Parallel shards:
  - `official-package`: author the exact official `skiff.run/registry` package in
    skiff-packages using canonical public DTO/wrapper source and compiler-owned injected native
    declarations. Expose only 21 operations including atomic `activation.activate`; no public
    prepare/commit/abort, paths, JSON, bytes, or manifest capability/binding strings.
  - `capability-projection`: derive `PackageRuntimeRequirements` by scanning lowered FileIR exact
    native binding keys and the canonical native spec mapping. Deduplicate/sort exact capability
    id=`skiff.registry.trusted`, version=`1` and operation scopes; deployment binding must be exact.
    Ordinary packages, package names, manifest/user strings, or service declarations grant nothing.
- Worktree: shard-specific worktree in the relevant integration repository.
- Validation: focused positive/parity/negative tests, including forged package id, missing/unknown
  binding, ordinary package, and removed manifest requires. No cross-shard edits, stable, full gate,
  merge, push, compatibility, or duplicate signature/capability tables.

