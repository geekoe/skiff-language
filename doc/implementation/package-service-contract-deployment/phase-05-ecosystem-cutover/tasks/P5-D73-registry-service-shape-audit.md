# P5-D73 Registry service shape audit

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at commit `335957b`.
- Read-only repositories: current skiff-packages and current Internals service patterns.
- Determine the canonical files/manifests/API/source/test/deployment shape for an official ordinary
  `skiff.run/registry` service located in skiff-packages. Map the existing four record/pointer/CAS
  semantics into ordinary ServiceContract operations and `std.db` storage without native/compiler
  privilege.
- Return bounded implementation DAG, source ownership, exact operation/type surface, DB requirements,
  positive/negative tests and how callers discover the service contract. Do not invent a client
  package or put URL/provider config in the service.
- No edits, installs, commits, stable, or full gate.

