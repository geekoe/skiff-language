# P5-D68 Package runtime residual audit

- Authority: `doc/architecture/package-service-contract-deployment.md`, canonical package tests and
  exact callable effects.
- Candidate: current Skiff/skiff-packages integrations after F76/F78/F80.
- Parallel read-only shards:
  - `openai-next`: direct production+overlay trace to the first remaining exact unknown target after
    JsonObject.set and std.json.decode; return runtime-proven semantics and owner.
  - `session-config`: trace canonical test config ownership for
    `skiff.run/http-session.cookieName` and why track's dependency fixture sees the same missing
    binding. Determine whether package.yml test config, test-runner dependency configuration, or
    both are required; enumerate all required config keys.
  - `aliyunoss-http`: trace the three passing pure cases and three HTTP 500 assertion failures through
    canonical fake HTTP expectations/request shape. Identify exact mismatch (method/url/headers/body/
    response expectation/capability) for all three without changing runtime.
- No edits, installs, commits, stable, full gate, or parallel runtime builds. Use compile-only/static
  probes and at most one serial isolated runtime where essential. Return bounded fix tasks.

