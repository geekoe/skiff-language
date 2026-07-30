# P5-F71 External official source authority checkpoint

- Authority: `doc/architecture/package-service-contract-deployment.md`, official package repository
  ownership and compiler-owned trusted registry native declarations.
- Candidate: current Skiff Phase 5 integration; D63 proved the implicit Skiff-root registry owner
  cannot authorize the external canonical skiff-packages source.
- Worktree: `skiff-p5-f71-external-official-authority` from current Skiff integration.
- Write owner: compiler input opaque official-package authority contract, canonical path/provenance
  validation, and Skiff trusted tooling/CLI transport that resolves an official-source binding
  before invoking compiler authoring.
- Required outcome: exact `skiff.run/registry` authority binds canonical external package root,
  canonical `package.yml`, and the trusted authority descriptor/config identity. Package id, cwd,
  manifest, environment, or a naked package-root flag cannot construct authority. Reject copies,
  symlink escape/TOCTOU, mismatched id/root/manifest, duplicate owner, and missing binding.
- Remove implicit discovery/authorization of `<skiff-root>/registry`; lack of external binding must
  fail closed. Preserve std/prelude authority and ordinary package behavior.
- Non-goals: authoring the skiff-packages registry source, changing skiff-packages scripts, native
  signatures/projection, stable access, compatibility.
- Validation: real trusted-tooling positive using an explicit test binding plus all rejection probes;
  ordinary forged `skiff.run/registry`, std/prelude regression, formatting/check/tests.
- Deliver one commit/evidence; do not merge or push.

