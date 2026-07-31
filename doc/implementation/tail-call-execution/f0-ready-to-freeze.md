# F0 tail-call gate ready-to-freeze result

Status: `F0_READY_TO_FREEZE`

This result establishes pre-freeze command and environment readiness. It is not
an implementation acceptance verdict, does not itself freeze a candidate, and
does not contain G1 evidence.

## Authority, role, and exact input

The direct parents are
[`f0-post-r2-gate-preflight.md`](./f0-post-r2-gate-preflight.md),
[`f0-gate-preflight.md`](./f0-gate-preflight.md), and
[`r2-ordinary-stack-regression.md`](./r2-ordinary-stack-regression.md). Their
trace continues through
[`parent-checkpoint.md`](./parent-checkpoint.md) to the canonical tail-call
architecture and runtime reference contract.

F0 owns only gate-environment preparation, completion-evidence coverage, and
the unique post-R2 runtime/source selector rebuild. It does not implement or
accept production behavior and does not redefine tail-call semantics.

- Repository: `/Users/geek/workspace/skiff`
- Pre-freeze input commit:
  `069de63742dc05b5f495f1044ba6e40991377c07`
- Pre-freeze input tree:
  `2f1fbf2f4b51c5edeb760fdae428602becf0825a`
- Gate branch/worktree: `codex/tco-final-gate-2` /
  `/Users/geek/workspace/skiff-tco-final-gate-2`
- Integration owner: `/root/tco_integrator`
- Gate owner through G1: `/root/tco_final_gate_2`
- Date/timezone: 2026-07-31, Asia/Shanghai

The worktree was created directly from the exact input and stayed
tracked-clean through dependency preparation, plan enumeration, and both
selectors. The difference from the prior post-R2 code candidate
`7ee8b2751ba51d50271f12dde6b0d92aca9f5b20` is only
`f0-post-r2-gate-preflight.md`; no production, test, lock, manifest, or
configuration file changed.

## Environment, cache, and capacity

| Item | Identity |
| --- | --- |
| physical source root | `/Users/geek/workspace/skiff-tco-final-gate-2` |
| Node | `v25.9.0` |
| pnpm | `10.29.2` |
| Cargo | `1.88.0 (873a06493 2025-05-10)` |
| rustc | `1.88.0 (6b00bc388 2025-06-23)` |
| pnpm store | `/Users/geek/Library/pnpm/store/v10` |
| exclusive Cargo target | `/Users/geek/workspace/skiff/build/cargo-target` |
| Cargo target filesystem identity | device `16777229`, inode `217447685`, owner `geek:staff` |

The main Agent assigned the existing Cargo target exclusively to this Gate
owner through G1 after a host-capacity audit. The integrator confirmed it would
not run Cargo/verify or touch this target. F0 did not clean, delete, replace, or
move the cache.

| Capacity point | Data volume | Available | Cargo target |
| --- | --- | --- | --- |
| before gate preparation | 460 GiB total, 404 GiB used, 98% capacity | 11 GiB (`12,054,916` KiB) | 62 GiB |
| after runtime selector | 460 GiB total, 406 GiB used, 98% capacity | 9.6 GiB | 64 GiB |
| after source selector and cleanup | 460 GiB total, 407 GiB used, 98% capacity | 8.6 GiB (`8,972,116` KiB) | 65 GiB |

The cache retained the same filesystem identity throughout. Both assigned
selectors completed on it, so the earlier independent-cache
`No space left on device` result did not recur. This prepared cache and the
gate worktree's ignored dependency links remain reserved and must not be
cleaned or mutated before G1.

Process scans before dependency preparation, immediately before the runtime
selector, between the two selectors, and after source-selector cleanup found no
competing `pnpm verify`, `scripts/verify.mjs`, `run-skiff-tests.mjs`, Cargo, or
rustc process. The source selector created only its canonical temporary
isolated Mongo/router/runtime instance and removed it after the tests. No
stable instance or live selector was touched.

## Frozen-lockfile dependency preparation

F0 ran `pnpm install --frozen-lockfile` once in each package directory:

| Working directory | Result |
| --- | --- |
| `router` | pass; lock current; 66 packages reused |
| `scripts` | pass; lock current; 4 packages reused |
| `telemetry` | pass; lock current; 68 packages reused |
| `vscode` | pass; lock current; 300 packages reused |

pnpm reported ignored lifecycle scripts for telemetry's `esbuild@0.27.7` and
VS Code's `@vscode/vsce-sign@2.0.9` and `keytar@7.9.0`. F0 did not approve
scripts or change mutable pnpm policy. The installs created only ignored
worktree-local `node_modules` trees.

Tracked and staged diffs remained empty. Each working lock blob equaled the
blob at `HEAD`:

| Lockfile | Git blob | SHA-256 |
| --- | --- | --- |
| `Cargo.lock` | `97d50b9ad60ea889b3bb37ff8b50a6e2bba90ada` | `098b922ed192e199028cd3f496ee30d0f7ae76b2a22e44738e3155b89c3079c1` |
| `router/pnpm-lock.yaml` | `d2edd11672dab8c5ba0aa18e6940b504996302ff` | `e10369257410556ba3bbcd210ccd8a8b03974468d958dafca26cdc6297ccc6ac` |
| `scripts/pnpm-lock.yaml` | `495f7d048830e8923bf58bb1f367932765e39d91` | `1720608d1e071c10f99762d318c3bbd02602a9121a1e99f1c479de971882cdf9` |
| `telemetry/pnpm-lock.yaml` | `1ec8b138692ab1b54a6cc083d2acdf870044727b` | `7ded5a40a51ee4ac7c53afba21790734ee301e9dfbec9736b2dbc52f6f5d264f` |
| `vscode/pnpm-lock.yaml` | `16ed0ab89bd9837b186f8d834b811812838908fa` | `3491e27a8c21128697e1e60cb9b9b601446cd9e34a6e5b3adc7a946a1e88b284` |

Read-only ESM resolution probes succeeded from the gate worktree for router
`vitest`, `typescript`, and `ws`; scripts `playwright` and `yaml`; telemetry
`vitest`, `typescript`, and `ws`; and VS Code `typescript` and `@vscode/vsce`.
This closes the prior missing-module prerequisite.

## Full-plan enumeration

F0 enumerated, but did not execute, the complete default non-live plan:

```bash
node scripts/verify.mjs --list
```

The command passed and expanded selector `verify` to 293 phases, including the
canonical source suite, seven runtime phases, all other implementation
subjects, Rust quality, type checks, JavaScript syntax checks, and default
checks. F0 did not run `pnpm verify`, a compiler selector, a live selector, or
any additional focused Cargo test.

## I2 and post-R2 selector evidence

The integrator-owned I2 evidence on
`7ee8b2751ba51d50271f12dde6b0d92aca9f5b20` remains applicable because the
only later baseline change before this F0 is the prior pure result document:

| Command | Result | Coverage |
| --- | --- | --- |
| `cargo check -p skiff-runtime-eval -p runtime` | pass; existing warnings only | merged R2 evaluator/runtime wiring |
| `cargo test -p runtime runtime_program_executes_bytes_natives_without_json_registry -- --nocapture` | pass; unchanged fixture | exact ordinary-expression regression |
| `cargo test -p skiff-runtime-eval tail_call_payload_does_not_inline_prepared_frame_into_flow -- --nocapture` | pass | fixed-size control-result boundary |
| `cargo test -p skiff-runtime-eval tail_call_transfer_accounts_like_the_corresponding_ordinary_call -- --nocapture` | pass | TCO accounting parity |
| `git diff --check 2438c61a38b77133bea1887904cde770b9c1d97b..HEAD` | pass | merged diff hygiene |

F0 then rebuilt the two previously missing selector results exactly once each
on the current pre-freeze input:

| Command | Result | Coverage |
| --- | --- | --- |
| `CARGO_TARGET_DIR=/Users/geek/workspace/skiff/build/cargo-target node scripts/verify.mjs --only runtime` | pass, exit 0; all six boundary phases and the canonical multi-package Rust phase passed; existing warnings and four expected ignored service-db live tests only | whole runtime subject after R2, including the unchanged bytes-native regression and tail-call suites |
| `CARGO_TARGET_DIR=/Users/geek/workspace/skiff/build/cargo-target node scripts/verify.mjs --only skiff-tests` | pass, exit 0; two canonical source entries passed in the isolated runtime | rebuilt real source/compiler/linker/runtime chain, including the 1,000-transfer source tail-call fixture |

The runtime selector was the sole retry after the prior capacity-aborted
attempt; that aborted Rust phase never established a candidate verdict. The
source selector was the sole post-R2 rebuild. Neither selector was rerun.

## Completion-evidence coverage

| Parent criterion | Final mapped evidence | F0 coverage result |
| --- | --- | --- |
| one legacy/assembly loop | C1 structure/search; V1 legacy and V2 assembly execution; final frozen production/diff search assigned to A1 | merged evidence and explicit final owner; no gap |
| direct/mutual/cross-module/generic/impl/branch | C1 exact return/link shape; V2 canonical assembly matrix; runtime selector | current evidence present |
| argument order, generic/self, heap carrier, return plans | V2 ordered-once arguments, generic/impl carriers, equal carrier and unequal-plan fallback; runtime selector | current evidence present |
| lexical and target negatives | R1/V1/V2 lexical barriers and `ValueBlock`/`PackageDirect` negatives; runtime selector | current evidence present |
| non-tail fuse and ordinary stack safety | V1 depth 32/33 guard and healthy follow-up; R2 fixed-size control result; I2 bytes-native probe; runtime selector | exact regression and whole-subject evidence pass |
| budget/deadline/stop and accounting | R1/V1 instruction-limit, deadline/stop, and finite accounting; I2 parity; runtime selector | current evidence present |
| stack bounds and bounded diagnostics | V1/R2 100,000-hop 1 MiB worker result and bounded error stack; runtime selector | current evidence present |
| scheduler depth | S1 production/seeded fresh-task depth evidence; runtime selector | R2 did not change scheduling owner; whole-subject evidence passes |
| real source chain | V3 1,000-transfer source fixture; fresh canonical `skiff-tests` selector | rebuilt on current input and passes |
| no duplicate or persisted mechanism | C1 scoped diff/search; A1 owns final frozen search | merged evidence and explicit final owner; no gap |

A1's frozen-candidate search and acceptance verdict remain deliberately
unexecuted at F0. They are the next independent evidence owner, not missing F0
evidence. G1 likewise remains reserved for the exact later frozen commit/tree.

## Result and freeze handoff

The exact input, dependency state, cache, capacity, process exclusivity,
completion-owner mapping, I2 evidence, and both post-R2 selectors are ready for
freeze. There is no known F0 blocker.

Required continuation:

```text
F0_READY_TO_FREEZE document integrated
  -> freeze one exact integration commit/tree
  -> independent A1 verdict on that frozen input
  -> Gate owner aligns this retained worktree exactly to the frozen input
  -> one and only one `pnpm verify` G1 run
```

The branch/worktree, ignored dependencies, and exclusive Cargo target are
intentionally retained. The Gate owner must not run G1 until the main Agent
provides the frozen integration commit/tree, and must not modify or repair that
candidate during G1.
