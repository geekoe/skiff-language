# P5-D75 Router service DB provisioning audit

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at `335957b`.
- Correct owner invariant: Router config `serviceDb.mongoUrl` is the only URL owner; Router forwards
  an activation-scoped binding to Runtime; runtime files/env/defaults and service artifacts cannot
  configure it.
- Read-only trace current canonical whole-assembly path from Router config/configActivation/protocol
  through runtime assembly admission, ActivationContext, Host DbProviderSource construction and
  request-scoped DbCapabilityContext. Include the package-test isolated Router path.
- Identify exact dropped payload/owner and the minimal Router/Runtime/Host/test-harness fanout.
  Reuse existing wire fields where present; do not add provider records or deployment URL fields.
- Return exact files, dependency order and own/dependency/retired/missing/URL-leak negatives. No
  edits, installs, commits, stable, full package run, or full gate.

