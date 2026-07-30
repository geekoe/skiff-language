# P5-F96 Registry service contract checkpoint

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at `335957b`.
- Repository/worktree: create `skiff-packages-p5-f96-registry-contract` from current skiff-packages
  integration.
- Write owner: replace erroneous `registry/package.yml`/authority tooling with ordinary
  `skiff.run/registry` service contract, `service.yml`, `api.yml`, `contract.yml`, deployment
  manifest skeleton and boundary model source. No storage implementation yet.
- Contract surface: exactly 20 unary operations—Put, Read, PointerRead, PointerCas, PointerHistory
  for each PackageArtifact, ServiceContract, ServiceDeployment and RuntimeAssembly. No activation,
  native binding, compiler authority, client package, common artifact union/kind or raw Json/bytes.
- Boundary types: contract-owned typed mirrors, path-free pointer keys/values, expected/candidate CAS,
  sequence receipts/history and strict typed errors. Service declares ordinary database state
  requirement `registry-store`; no URL/provider config.
- Validation: contract authoring/type-check, exact operation/schema negative probes, no native/
  authority surface. Do not implement std.db collections/CAS, edit Skiff, stable, merge, push or full
  gate. Deliver one commit/evidence.

