# P5-F445H-I6K-R1 Eval provider counter isolation

## 1. Parent, traceability, and DAG position

- Direct parent:
  `P5-F445H-I6K-independent-current-scope-acceptance-result.md`, blocking issue B1.
- The parent records its complete acceptance reading chain through
  `P5-F445H-I6R-current-scope-refresh-preflight-result.md`, the I6 consumer/result nodes,
  the phase plan, and the unique architecture source
  `doc/architecture/package-service-contract-deployment.md`.
- DAG node: close only I6 acceptance blocker B1, the Eval full-crate instability in the
  canonical provider-stream owner-zero test.
- Prerequisites: I6 current-scope production and combined probe are already integrated in the
  baseline; the independent acceptance has classified the failing selector as a test-isolation
  defect rather than a production leak.
- This node unblocks the Eval half of the I6 acceptance repair batch. It does not close Host
  blocker B2, accept I6, or unblock I7 by itself.
- Shared interface state: provider-stream production lifecycle semantics are frozen. This task
  may add test-only observation to the existing internal guard, but may not add a second lifecycle
  owner or change provider execution/publication behavior.

## 2. Baseline and repository identity

| Item | Value |
| --- | --- |
| Repository | `/Users/geek/workspace/skiff` |
| Baseline commit | `b5f991efc6b3dd191e8d73485aac03679fe6477c` |
| Baseline tree | `48d82258e3698055916ac5541f3764fe1e8a0bc1` |
| Branch | `codex/p5-f445h-i6k-r1-eval-counter` |
| Worktree | `/Users/geek/workspace/skiff-p5-f445h-i6k-r1-eval-counter` |
| Integration owner | `/root/phase05_integration_steward` |

The baseline is the exact independent-acceptance failure record. Evidence is invalidated by any
change to `runtime/eval`, its dependency graph, `Cargo.toml`, `Cargo.lock`, test scheduling, or the
commands recorded below.

## 3. Read-only preflight facts

- `PROVIDER_STREAM_TASKS_ACTIVE` is process-global in
  `runtime/eval/src/assembly_execution/async_stream_cancel.rs`.
- Production-spawned provider streams increment/decrement it through
  `ProviderStreamTaskGuard`; other Eval tests can legally own such a guard in parallel.
- `f445h_e4r_stream_provider_task_runs_real_terminal_publication_path` directly calls
  `run_provider_stream`, then asserts the process-global counter is absolute zero. The direct path
  does not currently install the guard, so the assertion neither identifies this case's task nor
  isolates it from concurrent tasks.
- The selector passes alone, while the acceptance full crate gate reported `418 passed / 1 failed`.
  This supports a fixture/observation isolation repair. It does not authorize masking a production
  leak.
- The complete mechanical owner set is the guard/direct-run implementation and the canonical
  current-scope unit fixture in the same module. No Host file or public API is required.

## 4. Write ownership and constraints

Expected writes:

- this task file;
- `runtime/eval/src/assembly_execution/async_stream_cancel.rs`;
- `runtime/eval/src/assembly_execution/async_stream_cancel/current_scope_tests.rs`;
- the matching result file created after implementation verification.

The expected list is not an absolute whitelist: a directly required test helper or mechanical
caller in the same Eval provider-stream test surface may be included if recorded in the result.
Stop and report instead if closure requires a shared runtime owner, production lifecycle change,
public/API behavior change, whole-crate serialization, suppression/ignore, or any Host blocker
change.

Explicit non-goals:

- do not weaken or delete owner-zero evidence;
- do not replace the full gate with the single selector;
- do not serialize the crate or all provider-stream tests;
- do not reset the global counter or wait for unrelated tasks to reach zero;
- do not change provider terminal/publication semantics;
- do not modify Host B2, stable/live services, network state, or MongoDB.

## 5. Implementation and observable completion

Implement a test-only per-task activity probe attached to the canonical fixture's
`ProviderStreamTask`. The real `run_provider_stream` lifecycle must install the existing guard so
both spawned and directly executed provider tasks traverse the same counter ownership boundary.
The guard must continue maintaining the process-global diagnostic counter and, only in test builds,
record that this exact task entered and subsequently returned its own active count to zero.

The canonical current-scope test must prove both:

1. its exact task entered the provider-stream guard once; and
2. after real typed terminal publication, that exact task has zero active owners.

The assertion must not depend on the instantaneous global count. Production/public behavior and
the existing global test-support diagnostic remain unchanged.

Completion criteria:

- the parent's baseline full Eval crate RED remains anchored to the unchanged production/test
  tree, and a direct pre-change rerun records whether the same nondeterministic schedule reproduces
  RED or instead demonstrates the contradictory green side of the isolation defect;
- the exact selector is non-zero and passes after the repair;
- one post-repair complete Eval crate gate passes with the full test count;
- locked package check, rustfmt check, and diff check pass;
- reverse search confirms no serialization/ignore/reset workaround and no Host write;
- implementation/tests and result are separate commits, both descended from the exact baseline.

## 6. Verification ownership

This developer owns exactly these commands in this worktree:

```bash
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-eval \
  f445h_e4r_stream_provider_task_runs_real_terminal_publication_path \
  --locked -- --nocapture
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-eval --locked --no-fail-fast
CARGO_NET_OFFLINE=true cargo check -p skiff-runtime-eval --locked
cargo fmt --check
git diff --check
```

The full Eval gate is run once after the repair. The baseline RED run is defect reproduction, not
candidate PASS evidence. No I6 combined, Host gate, stage gate, live, stable, network, or MongoDB
command belongs to this node.

Risk is medium test-evidence integrity: the code touched is an internal lifecycle guard, but the
intended behavior change is test-only observation. The earliest risk probe is the exact selector;
the authoritative closure evidence is the one complete Eval crate gate.

Candidate maturity remains pre-acceptance after this node. Integration of both B1 and B2 repairs,
a combined integration probe, and a new independent acceptance owner are still required before I6
can be accepted.
