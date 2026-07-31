# F0 tail-call gate preflight and failure classification

Status: `F0_BLOCKED`; candidate regression and missing gate dependency
preparation; no acceptance verdict and no full gate

## Authority, role, and frozen input

The direct parent is
[`parent-checkpoint.md`](parent-checkpoint.md). Its trace continues to the
canonical tail-call architecture and runtime reference contract. This F0 node
only owns freeze preparation, evidence coverage, and failure classification.
It does not accept the implementation, implement production behavior, or
redefine tail-call semantics.

Repository: `/Users/geek/workspace/skiff`

Candidate commit/tree:
`cd1146d3f2c60c85488d66e775c07a8df1edccb6` /
`6461caa73a81b6cb7501f74b7b44feef5a5c84a3`

Task-before baseline commit/tree:
`c34a954bca3580533c153d5761e8805c423dbb09` /
`8beb99c62fb2bf2f4fade9f41c855773c2e8a714`

Branch/worktree:
`codex/tco-f0-gate-preflight` /
`/Users/geek/workspace/skiff-tco-f0-preflight`

Integration owner: `/root/tco_integrator`. This leaf commits only this document,
hands it directly to the integration owner for merge and cleanup, and does not
push.

The candidate is a pre-acceptance checkpoint. A result of
`F0_READY_TO_FREEZE` means only that the exact code and environment are ready
for the independent A1 verdict and unique G1 full gate. It is not a PASS.

## Preflight contract and command ownership

F0 will enumerate, but not execute, each component selector and the complete
verification plan:

```bash
node scripts/verify.mjs --only compiler --list
node scripts/verify.mjs --only runtime --list
node scripts/verify.mjs --only skiff-tests --list
node scripts/verify.mjs --list
```

F0 will record the exact `node`, `pnpm`, and `cargo` identities, the physical
source root, clean tracked state, the target/cache owner, and whether another
full gate or shared-state mutation overlaps this preflight. Dependency
installation, live services, stable instance mutation, selector execution, and
`pnpm verify` are outside this node.

The V1 result in
[`v1-legacy-safety-pressure.md`](v1-legacy-safety-pressure.md) records a stack
overflow in
`runtime_program_executes_bytes_natives_without_json_registry` during its
unique runtime selector. F0 owns one meaningful classification, using the same
focused command and isolated target/cache environment on the exact candidate
and exact task-before baseline. It must distinguish:

- R1/TCO candidate regression: baseline passes while candidate fails, or the
  failure is otherwise causally tied to the candidate;
- genuinely pre-existing failure: both exact states reproduce the same failure
  under the same environment;
- environment/concurrency failure: results differ because toolchain, source,
  cache, process overlap, or runtime conditions are not equivalent.

F0 may run one smaller stack probe only if the identical focused comparison is
insufficient to classify the failure. It will not rerun the runtime selector.

## Completion-evidence coverage review

The merged candidate contains all leaf documents and their scoped evidence.
F0 maps the parent completion matrix to the following evidence owners; the
result section will record any identity or coverage gap discovered on the
merged tree.

| Parent criterion | Merged evidence | Evidence owner | F0 coverage check |
| --- | --- | --- | --- |
| one legacy/assembly loop | legacy direct/mutual/deep results; canonical assembly results; one-loop production search reserved for A1 | V1, V2, A1 | leaf evidence is merged and A1 has an explicit owner; no mapping gap |
| direct/mutual/cross-module/generic/impl/branch | exact `Return.value`/link target structure and canonical assembly matrix | C1, V2 | merged; no mapping gap |
| argument order, generic/self, heap carrier, return plans | ordered-once assembly arguments, generic/impl carriers, equal heap carrier, unequal-plan ordinary fallback | V2 | merged; no mapping gap |
| lexical/target negatives | non-tail/lexical tests plus assembly `ValueBlock` and `PackageDirect` negatives; R1 barrier/target source classification | R1, V1, V2 | merged; no mapping gap |
| non-tail fuse | depth 32 enters, depth 33 returns structured `programCallDepth`, followed by a healthy request | V1 | merged and repeated by I1; no mapping gap |
| budget/deadline/stop | finite accounting parity and infinite-tail `instructionLimitExceeded` | R1, V1 | merged; no mapping gap |
| stack bounds | 100,000-hop result on an actual 1 MiB worker stack and bounded 100,000-hop error diagnostic | V1 | merged focused evidence, but it does not excuse the independent ordinary-runtime stack regression below |
| scheduler depth | provider `tokio::spawn` observes fresh callable depth while error export retains caller depth | S1 seeded evidence | merged; no mapping gap |
| real source chain | canonical isolated `skiff-tests` executes 1,000 source-authored tail transfers | V3 | merged; no mapping gap |
| no duplicate/persisted mechanism | C1 scoped diff/search plus independent frozen production/diff search | C1, A1 | C1 is merged and A1 has the final frozen-search owner; no mapping gap |

The merged-state I1 probe is expected to be owned by the integrator:

```bash
cargo check -p skiff-runtime-eval -p runtime -p skiff-compiler-lowering -p skiff-runtime-linker
cargo test -p runtime runtime_program_recursion_fails_before_exhausting_the_worker_stack
cargo test -p skiff-runtime-eval tail_call_projection_parity
```

F0 will not silently substitute its focused stack-overflow classification for
missing I1 evidence. Any missing or stale completion owner, merged-state probe,
or environment prerequisite keeps the candidate unfrozen.

## Stop and freeze classification

F0 returns `F0_BLOCKED` when the task-before baseline passes and the candidate
fails, the candidate has another relevant blocker, a required merged evidence
owner is absent/stale, or the gate environment cannot be made precise without
candidate mutation. The result must name invalid evidence, an exact minimum
reproduction, the repair owner, and the required DAG adjustment. F0 does not
repair it.

F0 returns `F0_READY_TO_FREEZE` only when the identical focused comparison
classifies the V1 stack overflow as genuinely pre-existing or otherwise
non-candidate, all completion criteria have merged evidence owners with no
blocking gap, selector expansion and full-plan enumeration work, and the exact
candidate/environment can be frozen without competing shared-state mutation.

## Merged evidence identity and I1

All scoped leaf commits below are ancestors of the candidate:

| Node | Merged implementation/evidence commit | Tree |
| --- | --- | --- |
| C1 | `3febcfcf1a93f36b6c197b012e2fb7b546133d26` | `7654aa47650e4f7e16d6ce6b1b3ebe941560bbb3` |
| S1 production | `ff5bea8b4e4feaa15e12e2a3979e24923c053a64` | `7103f9590479c20a2d006be9b9dd6d9880a3498d` |
| R1 | `62db55e1d85e00727faf7d28526dc9515745f4bb` | `9fbac2032a8d97e532792557b8fae97786d27ecf` |
| S1 seeded evidence | `345c9003b2256697850dc7d42ceda0ac4373e047` | `f77366086e87c9c5b22eb255b4bf4c0a12a8e15f` |
| V3 | `87f0561108dd24ec06953230fc04adda89a524ce` | `bdf0aac940dbc19827a01c7f85571e52955521c9` |
| V2 | `af9dffc90b472c442f38a83807c5a6e350a615b6` | `aa2c953d188929a9134297bea12f2e785df2885a` |
| V1 | `ad55b4418b8c566426e32b2db45dd772992c4b03` | `4175843b22ef1bceb5eeef83884bc82ef4b6b8bc` |

The integrator uniquely ran I1 in
`/Users/geek/workspace/skiff-tco-integration` at the exact candidate
commit/tree:

| Command | Result | Coverage |
| --- | --- | --- |
| `cargo check -p skiff-runtime-eval -p runtime -p skiff-compiler-lowering -p skiff-runtime-linker` | pass; existing warnings only | merged compiler/linker/runtime wiring |
| `cargo test -p runtime runtime_program_non_tail_recursion_fails_at_guard_and_stays_healthy -- --nocapture` | pass; 1 passed, 117 filtered | final replacement for the parent's provisional recursion filter |
| `cargo test -p skiff-runtime-eval tail_call_transfer_accounts_like_the_corresponding_ordinary_call -- --nocapture` | pass; 1 passed, 434 filtered | final replacement for the parent's provisional parity filter |
| `git diff --check c34a954bca3580533c153d5761e8805c423dbb09..HEAD` | pass | merged diff hygiene |

The parent explicitly allows provisional I1 filter names to resolve to final
leaf names. I1 therefore has no selector-name gap, but it does not execute the
bytes-native regression below and cannot mask that failure.

## Environment and plan result

Preflight was performed on 2026-07-31 in Asia/Shanghai with:

- Node `v25.9.0`;
- pnpm `10.29.2`;
- Cargo `1.88.0 (873a06493 2025-05-10)`;
- rustc `1.88.0 (6b00bc388 2025-06-23)`.

The exact candidate integration source root was
`/Users/geek/workspace/skiff-tco-integration`; it was clean at
`cd1146d3f2c60c85488d66e775c07a8df1edccb6` /
`6461caa73a81b6cb7501f74b7b44feef5a5c84a3`. F0's isolated candidate source
root was `/Users/geek/workspace/skiff-tco-f0-preflight`. The temporary baseline
source root was `/Users/geek/workspace/skiff-tco-f0-baseline-c34`; it was clean
at `c34a954bca3580533c153d5761e8805c423dbb09` before F0 removed the worktree.

Plan enumeration succeeded without executing a selector:

| Command | Result |
| --- | --- |
| `node scripts/verify.mjs --only compiler --list` | pass; 2 phases: compiler-boundary check and the canonical compiler Rust package set |
| `node scripts/verify.mjs --only runtime --list` | pass; 7 phases: three boundary check pairs and the canonical runtime Rust package set |
| `node scripts/verify.mjs --only skiff-tests --list` | pass; 1 phase: `node scripts/run-skiff-tests.mjs` |
| `node scripts/verify.mjs --list` | pass; complete default non-live `verify` plan enumerated 293 phases |

No `pnpm verify`, component selector, live selector, stable instance action, or
shared service mutation was run. Process snapshots before and after the focused
comparison found no other `pnpm verify`, `scripts/verify.mjs`, `cargo
test/check/clippy/build`, or `run-skiff-tests` process. The default plan is
non-live and does not require ownership of the stable Skiff instance.

F0 exclusively owned
`/Users/geek/workspace/skiff-tco-f0-target` and used it sequentially for both
focused states. `CARGO_TARGET_DIR` was otherwise unset. The comparison cache
was removed from the workspace after the run (moved recoverably to
`/Users/geek/.Trash/skiff-tco-f0-target-20260731`). The integration worktree's
default Cargo target resolves to
`/Users/geek/workspace/skiff-tco-integration/build/cargo-target`; its existing
6.5 GiB cache belongs to the integrator/I1 until explicitly handed to a future
unique gate owner. F0 did not write it.

The shared pnpm store resolves to `/Users/geek/Library/pnpm/store/v10`; F0 only
queried it. There are no worktree-local `node_modules` links in the isolated or
integration candidate worktree. A read-only dependency-resolution probe for
`router` could not resolve `vitest/package.json` or
`typescript/package.json`. Thus the full plan can be enumerated, but its
router/telemetry/VS Code phases are not executable in the current worktree
without a frozen-lockfile dependency preparation step. Because the candidate
is already code-blocked, F0 did not install dependencies into a disposable
worktree. The next preflight owner must prepare dependencies in the actual
future gate worktree before freezing and record the resulting source/cache
state.

## V1 failure classification

The exact focused comparison used the same command, toolchain, environment, and
F0-owned target cache on both isolated source states:

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-tco-f0-target \
  cargo test -p runtime \
  runtime_program_executes_bytes_natives_without_json_registry \
  -- --nocapture
```

| Source state | Result |
| --- | --- |
| candidate `cd1146d3f2c60c85488d66e775c07a8df1edccb6` / `6461caa73a81b6cb7501f74b7b44feef5a5c84a3` | fail, exit 101; the selected libtest thread reports stack overflow and aborts with SIGABRT |
| task-before `c34a954bca3580533c153d5761e8805c423dbb09` / `8beb99c62fb2bf2f4fade9f41c855773c2e8a714` | pass; 1 passed, 0 failed, 113 filtered |

The test and its bytes-native fixture predate this task; candidate blame for
the selected test remains at `c63b26fa`, and the candidate diff does not alter
that test or fixture. The relevant candidate change is therefore in the
integrated runtime/evaluator path, not a newly strengthened assertion. The V1
document reproduced the failure at `bc4fea09`, which already contains R1. That
comparison proves persistence after R1, not behavior before this task, so its
label “pre-existing” is invalid for F0.

This is an R1/TCO candidate regression, not an environment/concurrency failure.
The identical two-state result is sufficient; F0 did not run a larger-stack
probe and did not rerun `node scripts/verify.mjs --only runtime`.

## Result: `F0_BLOCKED`

The candidate must not freeze. It makes an unchanged ordinary bytes-native
runtime test stack-overflow where the exact task-before baseline passes. The
runtime selector already observed the same abort, so G1's default runtime phase
would predictably fail. In addition, worktree-local JavaScript dependencies
have not been prepared for the full default plan.

Invalid or insufficient evidence:

- V1's statement that the runtime-selector stack overflow is pre-existing is
  invalidated by the exact `c34a954b` PASS / candidate FAIL comparison.
- V1's runtime selector is a recorded failure, not passing freeze evidence.
- I1 remains valid for the commands it ran, but it does not cover this fixture
  and cannot supersede the failure.
- The focused V1/V2/V3 and S1/C1 leaf results remain evidence for their scoped
  criteria on this candidate; they do not establish whole-runtime regression
  safety or a gate-ready environment.

The minimum repair owner is a new R1 runtime-evaluator regression node, not F0,
V1, or the gate owner. Its exact reproduction is the one-test command above.
Its production write boundary starts with the R1 evaluator owners
`runtime/eval/src/eval_context.rs`,
`runtime/eval/src/program_execution.rs`, and
`runtime/eval/src/program_execution/tail_call.rs`; it must minimize the actual
write set after diagnosis and must not weaken or delete the unchanged
bytes-native test.

Required DAG adjustment:

```text
R2 ordinary-expression stack regression
  -> merged I2: bytes-native reproduction + affected TCO accounting/depth probe
  -> rebuild every R1/V1/V2/V3 dynamic result invalidated by R2's actual write set
  -> unique merged runtime selector
  -> dependency preparation in the future gate worktree
  -> new F0 preflight and exact candidate freeze
  -> A1 and G1
```

C1 remains valid unless compiler/linker owners change. S1 remains valid unless
the repair changes program-depth or provider scheduling owners. Any R1
production edit invalidates the corresponding R1/V1/V2/V3 dynamic evidence and
the current I1 runtime wiring/probes according to their leaf contracts; the
post-repair owner must determine the exact minimum rerun from its actual diff.
F0 does not authorize or implement that repair.
