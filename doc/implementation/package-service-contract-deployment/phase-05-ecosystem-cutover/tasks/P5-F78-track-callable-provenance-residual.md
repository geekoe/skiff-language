# P5-F78 Track callable provenance residual

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact callable provenance.
- Base checkpoint: F76 commit `2b474082358a6b329c57b25796a298c6918d8ef4`; do not start
  from integration without this predecessor.
- Worktree: create a new Skiff worktree/branch at that exact commit.
- Write owner: compiler/source callable provenance/transfer and focused tests only.
- Required outcome: trace `skiff.run/track:record` to the first exact remaining unknown target and
  add only runtime-proven canonical semantics/contextual transfer needed to remove
  `UnknownCallTarget`/spurious I/suspend. Preserve real DB/native suspend/capability facts and all
  unresolved/dynamic fail-closed behavior.
- Validation: source focused tests and the existing F76 combined ignored compile-only four-package
  probe. If the probe advances to an independent openai blocker, stop and report it; do not expand.
- Deliver commits including the F76 predecessor relationship and evidence; no artifact/boundary/
  package edits, stable, merge, push, compatibility, or full gate.

