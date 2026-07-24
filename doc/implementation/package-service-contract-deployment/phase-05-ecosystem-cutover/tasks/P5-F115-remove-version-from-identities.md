# P5-F115 Remove human version labels from identities

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §2–§5 and §10–§12.
- Confirmed decision: every human-readable `version` is a selector/label and participates in no identity
  computation.
- Blocker evidence: F111 found existing PackageBuildId, PackageLocalAbiIdentity and Deployment identity
  preimages still include version.

## DAG node

Remove every package/service/contract human version label from canonical identity preimages while retaining
version fields in coordinates, records, pointers and diagnostics.

## Write scope

- `artifact-identity` canonical preimages/validators/tests.
- Artifact/compiler/deployment callers and cross-system fixtures directly affected by recomputed identities.
- Focused structural checker preventing version fields from re-entering identity inputs.

Do not remove version labels from authoring records, change exact dependency selection, migrate Registry or
Internals, or change Router/Runtime topology.

## Required semantics

- Audit all PackageArtifact/build, PackageLocalAbi, Service API/protocol, ServiceDeployment and
  RuntimeAssembly identity constructors.
- Human package/service/contract version labels never enter a direct or nested identity preimage.
- Code/API/config/dependency changes that are semantically part of an identity remain relevant.
- Changing only a version label leaves the identity unchanged; records/pointer coordinates still preserve the
  changed label.
- No legacy identity compatibility path; Skiff is unpublished.

## Acceptance

- Mutation matrix covers every canonical identity family: version-only invariant and semantic-content
  sensitivity.
- Structural test enumerates actual identity preimage fields and rejects version-like keys.
- A generated service package/deployment fixture demonstrates unchanged identities under label-only change.
- Artifact identity, compiler/deployment focused tests and `git diff --check` pass.

Risk: high, ecosystem-wide content identity. No merge/push/stable.
