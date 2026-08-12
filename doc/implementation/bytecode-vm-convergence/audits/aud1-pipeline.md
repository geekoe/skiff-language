# AUD1: producer-to-entry pipeline

> Status: completed

## 1. Observed pipeline

The production path begins at compiler input and ends at the request entry:

```text
source facts
  -> compiler emission (`compiler/driver/pipeline/bytecode_lane.rs`)
  -> admitted handoff (`BytecodeCompilationHandoff::try_new`)
  -> immutable package record publication
  -> filesystem bytecode resolver
  -> deployment hydration
  -> `link_deployment`
  -> `verify`
  -> `DeploymentImage`
  -> `BytecodeRequestTarget`
  -> request entry
```

Evidence symbols:

- `compile_package` and `PackageCompileOutput` in `compiler/driver`.
- `emit_bytecode_artifact` in `compiler/emission`.
- `ValidatedBytecodeArtifact::admit` in `artifact-identity`.
- `publish_package_artifact_records_with_bytecode` in
  `compiler/driver/authoring/package_publication.rs`.
- `DeploymentBytecodeContentResolver` and `DeploymentBytecodeLoader` in
  `runtime/loader`.
- `FilesystemDeploymentBytecodeContentResolver` in
  `runtime/loader/src/filesystem_resolver.rs`.
- `link_deployment` in `runtime/linker`.
- `verify` and `VerifiedLinkedBytecodeImage` in `runtime/bytecode-verifier`.
- `DeploymentImage` and `PinnedDeploymentEntry` in `runtime/deployment-image`.

## 2. Authority observations

The current pipeline has multiple places that can rebuild or soften facts:

- Emitter derives lifecycle plans (`compiler/emission/src/bytecode/plans.rs`)
  instead of consuming source-owned plans directly.
- Linker has normalization fallbacks in `runtime/linker/src/bytecode`; the
  architecture review calls out `equivalent_type_ref` and lifecycle merging.
- Verifier mints `VerificationSeal` after independently proving linked code
  (`runtime/bytecode-verifier/src/verifier.rs`), while the review identifies
  cases where verifier-specific checks compensate for upstream loss.
- `DeploymentImageCache` in `runtime/deployment-image/src/cache.rs` caches by
  exact deployment owner, but host routing can still re-open deployment records
  through `BytecodeRoute` (`runtime/host/src/loader/bytecode_admission.rs`).

## 3. Phase 0 decision input

The target topology for Phase 1 is:

```text
source facts -> artifact -> structural admission -> exact linker -> immutable image
-> exact entry pin -> VM -> request response
```

The current verifier can be thinned after structural validation and exact
linking prove their obligations. Phase 1 must not rely on `VerificationSeal` as
an execution authority, and must not allow artifact bytes to be re-read while a
request executes.
