# P5-F62 Package test boundary closure

- Authority: `doc/architecture/package-service-contract-deployment.md`, official package artifact
  and canonical package-test boundary requirements.
- Predecessor: T07 at packages commit `ecb7485286fd4df6f2fed78022c75a2ad9c3cc36`
  and the assigned D59 finding.
- Each assigned shard owns exactly one surface:
  - `openai`: package fixture source only; fix the unresolved local `images` expression type, then
    collect and close further package-owned canonical boundary failures for both non-live cases.
  - `aliyunoss-projection`: Skiff compiler/test-overlay projection only; resolve legal calls from
    exported test cases to published production-package callable facts so they do not acquire the
    six conservative unknown effects. Preserve fail-closed behavior for genuinely unresolved calls.
- Worktree: create a shard-specific worktree from the relevant integration repository
  (`skiff-packages-phase-05-integration` for openai,
  `skiff-phase-05-integration` for aliyunoss projection).
- Non-goals: Host legacy closure, trusted-registry surfaces, stable instance, compatibility/fallback,
  other official packages.
- Validation: smallest overlay/boundary compiler probe including negative unresolved-call and
  caller-owned mutation coverage; canonical package command only if it can pass the known Host
  environment blocker without changing shared state.
- Deliver one commit and evidence; do not merge or push. If the next failure belongs outside the
  assigned owner, stop with the exact blocker instead of expanding scope.

