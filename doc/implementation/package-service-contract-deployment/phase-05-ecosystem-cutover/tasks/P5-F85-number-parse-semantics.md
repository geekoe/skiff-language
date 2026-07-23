# P5-F85 number.parse callable semantics

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact callable effects.
- Candidate: current Skiff integration; D68 traced OpenAI residual to `core.number.parse`.
- Worktree: create `skiff-p5-f85-number-parse-semantics` from current integration.
- Write owner: artifact-model canonical native callable semantics and focused compiler/source tests.
- Exact semantics: read-only string; return detached/fresh number or null; invalid/non-finite errors
  detached, never caller alias; W/R/T/E/I/U/S all false. Do not remove the real suspend fact from
  downstream HTTP request.
- Validation: registry mutation matrix, source wrapper probe, direct OpenAI production+overlay
  compile-only case0. Stop at next exact target if any.
- No package/runtime behavior edits, stable, merge, push, compatibility, or full gate.

