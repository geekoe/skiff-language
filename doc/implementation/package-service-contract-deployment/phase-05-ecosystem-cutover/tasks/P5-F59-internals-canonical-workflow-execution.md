# P5-F59 Internals canonical workflow execution

- Authority: `doc/architecture/package-service-contract-deployment.md`, canonical ecosystem
  workflow and four-object transaction requirements.
- Predecessor: T09D at Internals commit `ca4ca1c342dca4e26abc9e57247880c444908c93`;
  R09 found that package scripts only return a fixture plan and never execute the workflow.
- Repository/worktree: create `internals-p5-f59-workflow` from
  `/Users/geek/workspace/internals-phase-05-integration`.
- Write owner: canonical workflow runner, its package-script wiring, and focused isolated tests.
- Required outcome: `type-check` and `test` invoke a real runner which creates an isolated temporary
  ecosystem store, executes `withCanonicalAssembly`, publishes all contracts, compiles all packages
  independently, verifies deployments, resolves the single root assembly, consumes receipts, and
  cleans up. Missing, duplicate, partial, and failed stages must fail closed.
- Non-goals: service/client implementation, stable watch/reload, stable artifact roots, architecture
  changes, compatibility paths.
- Validation: focused workflow tests plus linked-worktree script probes with explicit
  `SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration`. Do not run build/dev/start.
- Deliver one commit and evidence; do not merge or push.

