# P5-F88 Package test config wiring

- Authority: `doc/architecture/package-service-contract-deployment.md`, canonical test-owned
  ServiceDeployment config.
- Predecessor: F86 current Skiff integration.
- Repository/worktree: create `skiff-packages-p5-f88-test-config` from current skiff-packages
  integration.
- Write owner: skiff-packages isolated test tooling and exact http-session/track test config
  declarations.
- Required values for exact `skiff.run/http-session` artifact:
  `cookieName` string `"app_session"` and `maxAgeSeconds` number `2592000`.
  Keep `cookieDomain` and `publicPaths` unbound.
- Required outcome: generate a temporary strict F86 literal input after dependency artifacts are
  known, pass it only to http-session and track canonical test runs, and clean it in finally.
  Track must bind the exact resolved dependency artifact, not package id/version text alone.
- Validation: cleanup/negative wrong-ref probe plus serial canonical `test:http-session` and
  `test:track`. No production manifest dependency config, stable, merge, push, or full gate.
- Deliver one commit/evidence and classify any later runtime assertion separately.

