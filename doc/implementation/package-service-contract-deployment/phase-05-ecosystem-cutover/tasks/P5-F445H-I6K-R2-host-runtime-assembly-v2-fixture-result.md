# P5-F445H-I6K-R2 Host runtime assembly v2 fixture repair result

Status:

```text
PASS
B2_CLOSED = YES
PRODUCTION_WRITES = 0
STRICT_VALIDATION_CHANGED = NO
```

The Host `active_runtime_assembly` fixture now gives its intentionally unknown
assembly reference the current strict v2 lexical identity. The reference remains
content-unknown, so the test reaches the intended resolver mismatch, asserts
`AssemblyActivationRejectReason::Resolve`, and preserves its committed-generation
and replica-isolation coverage. No compatibility or production behavior changed.

## 1. Identity and write set

| Item | Value |
| --- | --- |
| Baseline commit | `b5f991efc6b3dd191e8d73485aac03679fe6477c` |
| Baseline tree | `48d82258e3698055916ac5541f3764fe1e8a0bc1` |
| Branch | `codex/p5-f445h-i6k-r2-host-fixture` |
| Worktree | `/Users/geek/workspace/skiff-p5-f445h-i6k-r2-host-fixture` |
| Fixture/task commit | `067f8748eec50897c6f45588d7bbea7e4a15fd15` |
| Fixture/task tree | `6be5866a4bb05357af9f42e87a174e0a1edd9e91` |
| Network mode | `CARGO_NET_OFFLINE=true` for Cargo validation |

Actual fixture/task write set:

- `runtime/host/tests/active_runtime_assembly.rs`: one test-only identity prefix,
  v1 to v2.
- `P5-F445H-I6K-R2-host-runtime-assembly-v2-fixture.md`: execution contract.
- This result file is the only additional result-commit write.

There are no production, builder, golden, manifest, dependency, or `Cargo.lock`
writes. No Eval blocker file was touched.

## 2. RED and repair

Before repair, the exact non-zero selector was:

```bash
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-host \
  --test active_runtime_assembly \
  rejected_exact_ref_preserves_committed_generation_and_two_replicas_are_independent \
  --locked -- --exact --nocapture
```

It failed deterministically with:

```text
running 1 test
called `Result::unwrap()` on an `Err` value:
assemblyIdentity must use skiff-runtime-assembly-v2:sha256:<64 lowercase hex>
test result: FAILED. 0 passed; 1 failed; 1 filtered out
```

The repair changed only the unknown reference from
`skiff-runtime-assembly-v1:sha256:<64 b>` to
`skiff-runtime-assembly-v2:sha256:<64 b>`. The digest still differs from the
fixture assembly, so this does not convert the negative case into a resolved
reference or weaken validation.

## 3. GREEN and full gate

| Level | Command | Result | Coverage |
| --- | --- | --- | --- |
| Exact selector | RED command above, after repair | PASS; `1 passed / 0 failed`, one filtered | Strict-v2 input reaches intended Resolve rejection and state assertions |
| Host full crate gate, run once after repair | `CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-host --locked --no-fail-fast` | PASS; `328 + 2 + 6 + 2 + 1(doc) = 339 passed / 0 failed` | Host unit, integration, convergence, and doc targets |
| Locked check | `CARGO_NET_OFFLINE=true cargo check -p skiff-runtime-host --locked` | PASS; existing warnings only | Host dependency/type wiring without lockfile mutation |
| Format | `cargo fmt --check` | PASS | Rust formatting |
| Whitespace | `git diff --check` | PASS | Patch hygiene |
| Reverse search | `rg -n 'skiff-runtime-assembly-v1' runtime/host/src runtime/host/tests/active_runtime_assembly.rs` | no matches | Host production and repaired fixture contain no v1 identity |

The old Host gate result was `338 passed / 1 failed`; the repaired result is
`339 passed / 0 failed`. The full gate was not rerun after its single GREEN run.

## 4. Self-acceptance matrix

| Task clause | Code evidence | Reverse-search evidence | Dynamic evidence |
| --- | --- | --- | --- |
| Unknown reference is lexically valid strict v2 | `active_runtime_assembly.rs` constructs `skiff-runtime-assembly-v2:sha256:<64 b>` | no v1 in Host production or repaired fixture | exact selector PASS |
| Resolver rejection semantics remain intact | unchanged assertion matches `AssemblyActivationRejectReason::Resolve` | diff contains no production validation/resolver edits | exact selector and Host full gate PASS |
| Committed generation and replicas remain independent | unchanged registration equality and second-replica assertions | actual code diff is one test string | full `active_runtime_assembly` target `2/2` PASS |
| No compatibility or strictness relaxation | zero production writes; prefix moved forward to v2 | Host production contains no v1 generator | locked check and full gate PASS |
| B2 Host gate blocker closed | deterministic RED becomes GREEN | old fixture token absent | Host full crate `339/339` PASS |

## 5. Boundary and handoff

```text
B2_CLOSED
```

This result closes only acceptance blocker B2. Eval blocker B1 remains outside
this task, so this branch does not declare `I6_ACCEPTED` or `I7_UNBLOCKED`.
Integration must be performed by `/root/phase05_integration_steward`; this
development owner did not merge, clean the first-level worktree/branch, push,
start stable/live services, use network, or access MongoDB.
