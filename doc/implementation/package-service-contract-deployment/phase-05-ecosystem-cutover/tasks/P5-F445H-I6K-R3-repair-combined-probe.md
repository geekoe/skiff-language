# P5-F445H-I6K-R3 repair combined integration probe

## 1. Parent, traceability, and DAG position

- Direct parent:
  `P5-F445H-I6K-independent-current-scope-acceptance-result.md`, whose B1 and B2
  blocked I6 acceptance.
- Repair inputs:
  `P5-F445H-I6K-R1-eval-provider-counter-isolation{,-result}.md` and
  `P5-F445H-I6K-R2-host-runtime-assembly-v2-fixture{,-result}.md`.
- Prior combined evidence:
  `P5-F445H-I6J-current-scope-combined-probe-result.md`.
- The parent chain continues through the I6 refresh and consumer results, the
  Phase 05 plan, and the unique architecture source
  `doc/architecture/package-service-contract-deployment.md`.
- DAG node: after both repair branches have merged, run the one cheap combined
  integration probe required before a new independent I6 acceptance.
- This node may mark the repair batch ready for re-acceptance. It cannot accept
  I6, unblock I7, or replace the independent four-crate acceptance.

## 2. Exact candidate identity

| Item | Value |
| --- | --- |
| Repository | `/Users/geek/workspace/skiff` |
| Baseline commit | `55992a4d494170f3fe846ea1a22dc1154beeafbe` |
| Baseline tree | `48b2812b59da4083483493de72ab0437be2ce074` |
| Branch | `codex/p5-f445h-i6k-r3-combined` |
| Worktree | `/Users/geek/workspace/skiff-p5-f445h-i6k-r3-combined` |
| Integration owner | `/root/phase05_integration_steward` |
| Network mode | `CARGO_NET_OFFLINE=true` |
| Cargo target | worktree-local `build/cargo-target` |

Read-only preflight established that the baseline contains both repair
implementation/result histories:

- R1 implementation `f6eb9d4b017f57536b1fdf3186f7540669049300`
  is an ancestor and installs the exact-task activity probe in the real
  provider-task guard path.
- R2 implementation `067f8748eec50897c6f45588d7bbea7e4a15fd15`
  is an ancestor and changes only the stale Host integration fixture identity
  from strict v1 to strict v2.
- Relative to the common repair baseline
  `b5f991efc6b3dd191e8d73485aac03679fe6477c`, the only non-document changes are
  the two Eval provider-task files and one Host integration fixture.

Any production, test, fixture, manifest, dependency, lockfile, or environment
change after this baseline invalidates the affected evidence.

## 3. Scope, ownership, and non-goals

Allowed writes are only this task file and its matching result file. Do not
modify production, tests, fixtures, Cargo manifests, `Cargo.lock`, generated
sources, configuration, or external state.

This is an integration evidence node, not a development or independent
acceptance node:

- do not repair a failure; classify it and record `FAIL`;
- do not rerun the complete Eval or Host crate gates already owned by R1/R2;
- do not rerun all 12 I6J selector groups;
- do not run a full I6/stage gate, stable/live service, network operation, or
  MongoDB operation;
- do not merge, clean another first-level worktree/branch, push, or change the
  shared integration branch.

Stop with `TASK_NOT_EXECUTABLE` before dynamic work if the candidate identity is
wrong, either repair is absent, a required selector no longer exists, or a
non-document write is already present.

## 4. Cheap probe matrix and exact commands

Every test selector must first be listed and prove a non-zero count. Listing and
execution must match.

### R1 direct failure path

```bash
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-eval \
  f445h_e4r_stream_provider_task_runs_real_terminal_publication_path \
  --locked -- --list
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-eval \
  f445h_e4r_stream_provider_task_runs_real_terminal_publication_path \
  --locked -- --nocapture
```

Expected: one listed and one passed. This proves the canonical fixture enters
the real provider-task guard once and returns its exact per-task active owner to
zero after typed terminal publication.

### R2 direct failure path

```bash
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-host \
  --test active_runtime_assembly \
  rejected_exact_ref_preserves_committed_generation_and_two_replicas_are_independent \
  --locked -- --exact --list
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-host \
  --test active_runtime_assembly \
  rejected_exact_ref_preserves_committed_generation_and_two_replicas_are_independent \
  --locked -- --exact --nocapture
```

Expected: one listed and one passed. This proves the intentionally unknown but
lexically valid strict-v2 reference reaches the intended Resolve rejection and
preserves committed generation and replica isolation.

### Representative I6 current-scope path

```bash
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-eval \
  f445h_i6_carrier_delivery_receipt \
  --locked -- --list
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-eval \
  f445h_i6_carrier_delivery_receipt \
  --locked -- --nocapture
```

Expected: five listed and five passed. This is the smallest already-checked-in
I6J representative group spanning current-carrier delivery to HTTP unary,
WebSocket request, time, file, and Actor lower APIs. It is not a replacement for
the remaining I6J groups or independent acceptance.

### Shared locked wiring and hygiene

```bash
CARGO_NET_OFFLINE=true cargo check \
  -p skiff-runtime-eval -p skiff-runtime-host --locked
cargo fmt --check
git diff --check
```

The locked check is intentionally limited to the two repaired packages. It
proves their merged type/dependency wiring without duplicating either full crate
gate.

## 5. Static checks and failure classification

Before recording the verdict:

- confirm the per-task probe, `ProviderStreamTaskGuard::for_task`, and the
  canonical exact-task assertions remain present;
- confirm the repaired Host fixture uses strict v2 and no v1 identity remains
  in Host production or that fixture;
- confirm the baseline-to-final non-document diff is empty;
- confirm no ignore, whole-crate serialization, global counter reset, strict-v2
  relaxation, or compatibility fallback entered through integration.

Classify a failure as one of:

1. candidate identity/integration mismatch;
2. R1 direct-path regression;
3. R2 direct-path regression;
4. representative I6 current-scope regression;
5. locked wiring/build regression;
6. environment/tooling failure.

Record the first failure plus any already-collected independent result, then
stop. Do not modify code or expand into a full gate.

## 6. Completion and handoff

PASS requires:

- exact baseline identity and both repair commits present;
- selector listing `1 + 1 + 5 = 7` and execution `7/7`;
- Eval+Host locked check PASS;
- rustfmt and diff checks PASS;
- only task/result documentation writes;
- a committed result and clean worktree.

On PASS report:

```text
READY_FOR_I6_REACCEPTANCE = YES
```

This advances the merged repair batch only to a re-acceptance-ready
pre-acceptance candidate. A new independent owner must still rebuild the I6
acceptance verdict on the resulting exact code state.
