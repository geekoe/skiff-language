# P5-F94B Privilege removal lock and std pin

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at `335957b`.
- Candidate: current Skiff integration after F94; I94 found only stale Cargo.lock entry and std public
  symbol count 98→99.
- Worktree: create `skiff-p5-f94b-lock-std-pin`.
- Write owner: regenerate/check Cargo.lock and classify the exact 99th std public symbol versus the
  pre-F94 baseline. Update the pin only if the symbol is an already-authoritative std export unrelated
  to registry privilege removal; otherwise repair the leak.
- Validation: I94 affected tests/reverse searches with low-disk settings. No Router TS migration,
  Registry service code, stable, merge, push or full gate.
- Deliver one commit/evidence.

