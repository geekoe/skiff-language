# F0 post-R2 gate preflight

Status: `F0_BLOCKED`; the unique runtime selector exhausted the gate host
filesystem before its Rust phase completed. This result is not an acceptance
verdict, does not freeze the candidate, and does not authorize G1.

## Authority, role, and candidate

The direct parents are
[`f0-gate-preflight.md`](./f0-gate-preflight.md) and
[`r2-ordinary-stack-regression.md`](./r2-ordinary-stack-regression.md). Their
trace continues through
[`parent-checkpoint.md`](./parent-checkpoint.md) to the canonical tail-call
architecture and runtime reference contract.

F0 owns only gate-environment preparation, evidence coverage, and the unique
post-R2 runtime/source selector rebuild. It does not implement or accept
production behavior and does not redefine tail-call semantics.

- Repository: `/Users/geek/workspace/skiff`
- Pre-acceptance candidate commit:
  `7ee8b2751ba51d50271f12dde6b0d92aca9f5b20`
- Pre-acceptance candidate tree:
  `de1d3c78b44656750ec65ed8b6acd12342f8240b`
- Gate branch/worktree: `codex/tco-final-gate` /
  `/Users/geek/workspace/skiff-tco-final-gate`
- Integration owner: `/root/tco_integrator`
- Date/timezone: 2026-07-31, Asia/Shanghai

The worktree started at the exact candidate and remained tracked-clean through
dependency preparation and the failed selector. F0 did not modify production
code, lockfiles, configuration, services, or the stable local instance.

## Environment and dependency preparation

Tool and source identities:

| Item | Identity |
| --- | --- |
| physical source root | `/Users/geek/workspace/skiff-tco-final-gate` |
| Node | `v25.9.0` |
| pnpm | `10.29.2` |
| Cargo | `1.88.0 (873a06493 2025-05-10)` |
| rustc | `1.88.0 (6b00bc388 2025-06-23)` |
| pnpm store | `/Users/geek/Library/pnpm/store/v10` |
| transferred Cargo target/cache | `/Users/geek/workspace/skiff-tco-integration/build/cargo-target` |

The prior F0 found that the candidate worktree had no JavaScript dependency
links. Before any freeze, this F0 ran `pnpm install --frozen-lockfile`
sequentially from each package directory:

| Working directory | Result |
| --- | --- |
| `router` | pass; lockfile current; 66 packages reused |
| `scripts` | pass; lockfile current; 4 packages reused |
| `telemetry` | pass; lockfile current; 68 packages reused |
| `vscode` | pass; lockfile current; 300 packages reused |

pnpm reported ignored lifecycle scripts for telemetry's `esbuild@0.27.7` and
VS Code's `@vscode/vsce-sign@2.0.9` and `keytar@7.9.0`; no approval or mutable
policy change was made. The installs created only ignored worktree-local
`node_modules` trees. Tracked state stayed clean, and the candidate commit/tree
did not change.

The tracked lock identities were unchanged after installation:

| Lockfile | Git blob | SHA-256 |
| --- | --- | --- |
| `Cargo.lock` | `97d50b9ad60ea889b3bb37ff8b50a6e2bba90ada` | `098b922ed192e199028cd3f496ee30d0f7ae76b2a22e44738e3155b89c3079c1` |
| `router/pnpm-lock.yaml` | `d2edd11672dab8c5ba0aa18e6940b504996302ff` | `e10369257410556ba3bbcd210ccd8a8b03974468d958dafca26cdc6297ccc6ac` |
| `scripts/pnpm-lock.yaml` | `495f7d048830e8923bf58bb1f367932765e39d91` | `1720608d1e071c10f99762d318c3bbd02602a9121a1e99f1c479de971882cdf9` |
| `telemetry/pnpm-lock.yaml` | `1ec8b138692ab1b54a6cc083d2acdf870044727b` | `7ded5a40a51ee4ac7c53afba21790734ee301e9dfbec9736b2dbc52f6f5d264f` |
| `vscode/pnpm-lock.yaml` | `16ed0ab89bd9837b186f8d834b811812838908fa` | `3491e27a8c21128697e1e60cb9b9b601446cd9e34a6e5b3adc7a946a1e88b284` |

Read-only resolution probes then resolved package-local dependencies from the
gate worktree, including router `vitest`, `typescript`, and `ws`; scripts
`playwright` and `yaml`; telemetry `vitest` and `typescript`; and VS Code
`typescript` and `@vscode/vsce`. This closes the missing-module condition
reported by the prior F0, but it is preparation rather than candidate PASS
evidence.

The integrator explicitly transferred exclusive ownership of the Cargo cache
to this Gate owner at the exact candidate and reported no running
`cargo`/`rustc` process. It also transferred the merged I2 results and stopped
all further probe/cache activity. Immediately before the unique selector, a
process scan found no competing `pnpm verify`, `scripts/verify.mjs`,
`run-skiff-tests.mjs`, Cargo test/check/clippy/build, or rustc process. No live
service or shared stable-instance mutation overlapped F0.

## Full plan enumeration

F0 enumerated, but did not execute, the complete default non-live plan:

```bash
node scripts/verify.mjs --list
```

The command passed and expanded selector `verify` to 293 phases. It included
the canonical `skiff-tests` phase, the seven runtime phases, the other
implementation subjects, Rust quality, type checks, JavaScript syntax checks,
and default checks. F0 did not run `pnpm verify`, a compiler selector, a live
selector, or any V1/V2 focused test.

## Merged I2 and completion-evidence coverage

The integrator uniquely ran I2 on the exact clean candidate before transferring
the Cargo cache:

| Command | Result | Coverage |
| --- | --- | --- |
| `cargo check -p skiff-runtime-eval -p runtime` | pass; existing warnings only | merged R2 evaluator/runtime wiring |
| `cargo test -p runtime runtime_program_executes_bytes_natives_without_json_registry -- --nocapture` | pass; 1 test, unchanged fixture | exact F0 ordinary-expression regression |
| `cargo test -p skiff-runtime-eval tail_call_payload_does_not_inline_prepared_frame_into_flow -- --nocapture` | pass; 1 test | fixed-size owning control-result boundary |
| `cargo test -p skiff-runtime-eval tail_call_transfer_accounts_like_the_corresponding_ordinary_call -- --nocapture` | pass; 1 test | affected TCO accounting parity |
| `git diff --check 2438c61a38b77133bea1887904cde770b9c1d97b..HEAD` | pass | merged diff hygiene |

R2's bit-identical leaf evidence on the same tree also records 14 passing
`tail_call_` evaluator tests, one passing non-tail guard/health runtime test,
four passing legacy-tail runtime tests, package checks, rustfmt, and diff
hygiene.

The parent completion matrix remains mapped as follows:

| Parent criterion | Merged/scoped evidence | Post-R2 status before this F0 |
| --- | --- | --- |
| one legacy/assembly loop; no duplicate persisted mechanism | C1 structure/search, V1/V2 scoped evidence, final A1 frozen search owner | C1 unaffected by R2; final frozen search still belongs to A1 |
| direct/mutual/cross-module/generic/impl/branch; argument order; generic/self; heap carrier; return plans | V2 assembly matrix plus R2's bit-identical 14-test evaluator run | focused post-R2 evidence present |
| lexical/target negatives | R1/V1/V2 scoped tests and structure evidence, included in R2's affected evaluator run where selected | focused post-R2 evidence present |
| non-tail fuse and ordinary stack safety | V1 guard/health evidence, R2 unchanged bytes-native regression, merged I2 | exact regression and guard probes pass |
| budget/deadline/stop and instruction accounting | R1/V1 evidence, R2 evaluator run, merged I2 parity test | focused post-R2 evidence present |
| stack bounds and bounded diagnostic | V1 pressure evidence, R2 100,000-hop 1 MiB evaluator run and legacy-tail runtime run | focused post-R2 evidence present |
| scheduler depth | S1 production and seeded evidence | R2 did not change provider scheduling owner; final runtime selector still required |
| real source chain | V3 canonical 1,000-transfer source fixture | evaluator write invalidated dynamic result; canonical `skiff-tests` rebuild still required |

The focused I2/R2 evidence is necessary but cannot substitute for the assigned
merged runtime selector and source selector. Both had to pass before the
candidate could freeze.

## Unique selector result

F0 ran the assigned runtime selector exactly once:

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-tco-integration/build/cargo-target \
  node scripts/verify.mjs --only runtime
```

The six boundary phases passed:

1. runtime execution-boundary self-test;
2. runtime execution production boundary;
3. eval-error boundary self-test;
4. eval-error production boundary;
5. artifact-boundary self-test;
6. artifact production boundary.

The seventh phase, `implementation:runtime:rust`, invoked the canonical
multi-package `cargo test --no-fail-fast` command and failed with exit 101.
Several rustc processes reported `No space left on device` while writing
metadata, object files, fingerprints, and incremental output. The verify runner
therefore exited 101 before the Rust suite completed.

Immediately after failure, `/System/Volumes/Data` reported 460 GiB total,
416 GiB used, only 116 MiB available, and 100% capacity. The transferred Cargo
target had grown from 11 GiB before the selector to 12 GiB after it. No
`cargo`, rustc, or verify process remained. Candidate tracked state and all
recorded lock hashes were still unchanged.

Under the F0 fail-stop contract, the Gate owner did not rerun the runtime
selector and did not proceed to:

```bash
node scripts/verify.mjs --only skiff-tests
```

Thus there is no post-R2 source-selector result on this candidate.

## Result: `F0_BLOCKED`

This is an environment/capacity blocker, not evidence of a candidate semantic
failure. However, the runtime selector is incomplete and the source selector
was not run, so the pre-acceptance candidate cannot be frozen and G1 must not
start.

Still-valid evidence:

- candidate identity, tracked cleanliness, and unchanged lock identities;
- completed dependency installation and package resolution preparation;
- complete 293-phase plan enumeration;
- the six passing runtime boundary phases;
- merged I2 and the bit-identical scoped R2 evidence;
- unaffected C1 structure evidence and scoped leaf evidence within their
  recorded boundaries.

Missing or invalid freeze evidence:

- the runtime selector has no PASS because its Rust phase aborted;
- the post-R2 `skiff-tests` rebuild has no result;
- the failed-build cache cannot represent a prepared gate environment;
- no A1 frozen-candidate verdict or G1 full-gate evidence may be attached to
  this state.

Required DAG adjustment:

```text
F0_BLOCKED result integrated
  -> an explicit environment owner establishes sufficient disk capacity and
     prepares an isolated or explicitly transferred Cargo cache without
     changing candidate production code
  -> a new pre-freeze owner rechecks exact candidate/tree, tracked and lock
     state, dependencies, cache identity, and absence of competing mutation
  -> unique runtime selector PASS
  -> unique skiff-tests selector PASS
  -> new F0_READY_TO_FREEZE document and exact freeze
  -> A1 and G1
```

The Gate owner initially left the failed cache untouched. The host then lacked
enough space even to write this small result document. With explicit
authorization from the main Agent, the same exclusive cache owner ran:

```bash
cargo clean --target-dir /Users/geek/workspace/skiff-tco-integration/build/cargo-target
```

It removed 40,692 files and 14.4 GiB of reproducible Cargo output. The target
path was absent afterward and available disk space became 11 GiB. No source,
dependency store, user data, or other cache was removed. This recovery exists
only to make the blocked result committable; it does not repair or rerun any
evidence. The ignored dependency trees remain in the gate worktree for explicit
lifecycle disposition, but the worktree must not be treated as a frozen gate
environment.
