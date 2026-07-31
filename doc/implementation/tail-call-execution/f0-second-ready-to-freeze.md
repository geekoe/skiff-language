# F0 second-stability tail-call gate ready-to-freeze result

Status: `F0_READY_TO_FREEZE`

This result establishes the second stability cycle's pre-freeze command,
dependency, cache, capacity, and completion-evidence readiness. It is not an
implementation acceptance verdict, does not itself freeze a candidate, and
does not contain A2 or G2 evidence.

## Authority, parents, role, and exact input

The direct repository parent is
[`a1-finding-wave-checkpoint.md`](./a1-finding-wave-checkpoint.md). Its trace
continues through `parent-checkpoint.md` to:

1. [`../../architecture/tail-call-execution.md`](../../architecture/tail-call-execution.md);
2. [`../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety`](../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety).

The second direct input is the I3 integrator evidence transferred by
`/root/tco_integrator` for the exact merged candidate below. No separate I3
result file existed before this F0, so the exact commands, results, structure
assertions, and identities are fixed in this result rather than left only in
the handoff.

F0 owns gate-environment preparation, A1-blocker and criteria coverage review,
and the unique post-I3 runtime/source selector rebuild. It does not implement
or accept production behavior and does not redefine tail-call semantics.

- Repository: `/Users/geek/workspace/skiff`
- Pre-freeze input commit:
  `253665541a815209cb806d4354a20e79694778e3`
- Pre-freeze input tree:
  `4a80013ef2b1dc21b69fd6c8b7ccb2cb2b56f1fa`
- Finding-wave checkpoint:
  `0583e9e097d8883644e5b6e1fb4d21055cbd05d6`
- Gate branch/worktree: `codex/tco-final-gate-3` /
  `/Users/geek/workspace/skiff-tco-final-gate-3`
- Integration owner: `/root/tco_integrator`
- Gate owner through G2: `/root/tco_final_gate_3`
- Date/timezone: 2026-07-31, Asia/Shanghai

The worktree was created directly from the exact input and stayed
tracked-clean through dependency preparation, plan enumeration, and both
selectors. Only this result document is added by F0.

## I3 merged identity and evidence

The finding-wave integration line is:

| Node | Result/integration identity |
| --- | --- |
| T1 | `ee8adbd20dfbacf6e4f28e22f09e817035dd957f`, tree `cdd4b03a09b50f408e04e61b08434dfca633768c` |
| R3 | `be23c182f4161f68d9ad6acc111d64f69a0a99a3`, tree `105e22536b847349e46590814c3f214dc84ee9f1`; merged as `22c7c8ae28dc59a9f8282f26bef36f9f33e5c0e3` |
| E1 | `70ea262f04459998ac4b35851f2dbf490b26aa29`, tree `4719bc5dd9679bf8b62ba75700fb279f6bbc1669` |
| N1 | seven-commit test line ending at `997e8a37f8440af1b8b26c26229ab1862d3f3c41`, merged as the exact F0 input |

The integrator uniquely ran this combined probe from a clean
`/Users/geek/workspace/skiff-tco-integration` on the exact input:

| Layer | Command/evidence | Result and coverage |
| --- | --- | --- |
| merged compile | `CARGO_TARGET_DIR=/Users/geek/workspace/skiff/build/cargo-target RUSTFLAGS='-Dprivate_interfaces' cargo check -p skiff-runtime-eval -p runtime` | pass; public/private control wiring |
| E1 entry | `cargo test -p skiff-runtime-eval tail_call_entry_checkpoint -- --nocapture` | pass, 1/1; current tail edge is attributed exactly once when the next entry checkpoint fails |
| E1 carrier | `cargo test -p skiff-runtime-eval assembly_tail_call_carrier -- --nocapture` | pass, 2/2; nominal, union, representation, and container parity |
| E1 error | `cargo test -p runtime runtime_program_legacy_tail_call_error -- --nocapture` | pass, 2/2; bounded terminal error plus catch/rethrow identity and correlation |
| N1 assembly | `cargo test -p skiff-runtime-eval assembly_tail_call_negative -- --nocapture` | pass, 6; call argument, catch, service, Actor, native, and builtin |
| N1 owner barriers | final timeout, concurrent, DB transaction, DB lease, and stream filters | pass, 1 + 1 + 1 + 1 + 3; real deadline/arbitration/cleanup/defer owners |
| T1 | `/opt/homebrew/bin/node --test scripts/tests/command-execution-policy.test.mjs` | pass, 10/10; canonical 11 owners, 9 spawn, 2 execFile |
| hygiene | `git diff --check 0583e9e0..HEAD` | pass |

I3's structural assertions also passed: public `env::Flow` has exactly the six
baseline variants; `runtime/eval/src` has no `Flow::TailCall` or
`allow(private_interfaces)`; `program_execution.rs` contains the sole
production prepared-frame loop and one exact prepared-frame consumer; no
tail-hop spawn call was added; and the artifact model, runtime model, linked
program, config snapshot, manifests, dependencies, and lockfiles were
unchanged by the finding wave.

## A1 blocker closure and completion mapping

F0 re-read the authority, A1 checkpoint, all four leaf results, the current
production seam, and the test registrations. The three A1 blockers are closed:

| A1 blocker | Current closure |
| --- | --- |
| next entry loses `tailSite` | `PreparedTailCall` retains `caller` and `tail_site`; the sole trampoline promotes resolution and `prepare_program_executable_entry()` at that site before target-body execution. E1's distinct-site budget test passes. |
| internal control expands public `Flow` | public `Flow` is back to `Continue`, `Return`, `Break`, `LoopContinue`, `Parked`, and `ContinueConsumer`; crate-private `EvaluatorControl::TailCall(Box<PreparedTailCall>)` owns the internal transfer; strict I3 compilation and reverse searches pass. |
| required dynamic matrix is incomplete | E1 adds entry-site, carrier, and catch/rethrow evidence; N1 adds all missing real barrier and excluded-dispatch negatives. All final filters are non-zero and passed in their leaf evidence and merged I3 probe. |

The authority completion criteria now map to merged evidence as follows:

| Criterion | Final evidence on the input |
| --- | --- |
| exact eligibility and one legacy/assembly trampoline | C1 structure, V1/V2 positive execution, R3 crate-private control, I3 sole-loop search, current runtime selector |
| direct/mutual/cross-module/generic/impl/branch and ordered arguments | C1/V2 focused evidence, current runtime selector |
| generic/self and nominal/union/representation/container carriers; return-plan fallback | V2 plus E1 carrier matrix, current runtime selector |
| binary/wrapper/call-argument/catch/timeout/concurrent/DB/stream/service/Actor/native/builtin negatives | prior V1/V2 representatives plus N1's 13 real-owner tests, current runtime selector |
| non-tail fuse and ordinary stack safety | V1 guard/health and R2 bytes-native regression, current runtime selector |
| budget/deadline/stop, current-edge attribution, and accounting | V1/R2 accounting plus R3/E1 entry-site closure and N1 owner barriers, current runtime selector |
| bounded stack and exception identity/correlation | V1 pressure evidence and E1 100,000-hop catch/rethrow evidence, current runtime selector |
| scheduler fresh depth | S1 production and seeded-depth evidence, current runtime selector |
| real source chain | V3 source fixture and the current canonical `skiff-tests` selector |
| no public, duplicate, persisted, configured, or spawned alternate mechanism | C1/R3/I3 structural and diff searches |
| tooling cardinality defect from the first G1 | T1 10/10 focused evidence, unchanged policy assertions, and I3 focused rerun |

The leaf result statuses that still say “awaiting integration” are historical
wording only: their exact commits above are ancestors of the clean input and
the integrator's merged probe covers their final filters. There is no remaining
completion-owner or executable-evidence gap.

## Environment, cache, capacity, and exclusivity

| Item | Identity |
| --- | --- |
| physical source root | `/Users/geek/workspace/skiff-tco-final-gate-3` |
| Node | `v25.9.0` |
| pnpm | `10.29.2` |
| Cargo | `1.88.0 (873a06493 2025-05-10)` |
| rustc | `1.88.0 (6b00bc388 2025-06-23)` |
| pnpm store | `/Users/geek/Library/pnpm/store/v10` |
| exclusive Cargo target | `/Users/geek/workspace/skiff/build/cargo-target` |
| Cargo target filesystem identity | device `16777229`, inode `217447685`, owner `geek:staff` |

The integrator transferred exclusive ownership of the existing Cargo target
after I3 and committed to run no Cargo, rustc, selector, verify, or cache
mutation during F0. F0 did not clean, delete, replace, or move it.

| Capacity point | Data volume | Available | Cargo target |
| --- | --- | --- | --- |
| before preparation | 460 GiB total, 405 GiB used, 98% capacity | 11 GiB | 63 GiB |
| before source selector, after runtime | 460 GiB total, 407 GiB used, 99% capacity | 8.2 GiB | 66 GiB |
| after both selectors and isolated cleanup | 460 GiB total, 409 GiB used, 99% capacity | 6.9 GiB (`14,568,952` 512-byte blocks) | 67 GiB |

The target kept the same device/inode throughout. The cache now contains the
successful current runtime and source builds; 6.9 GiB remains available.
G2 must recheck this exact identity and capacity immediately before the full
gate and fail-stop without running if the environment changed materially.

Process scans before preparation, before each selector, between selectors, and
after isolated cleanup found no competing `pnpm verify`,
`scripts/verify.mjs`, `run-skiff-tests.mjs`, Cargo, or rustc process. The
source selector created its canonical temporary Mongo/router/runtime instance
on dynamic `46043`-`46046` ports and stopped it afterward. It did not mutate
the stable instance. The integration input remained clean and the integrator
reported no further candidate writes; all finding-wave implementation agents
were complete. The only F0 write is this result document.

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

Tracked and staged diffs stayed empty. Every working lock equaled its `HEAD`
blob after both selectors:

| Lockfile | Git blob | SHA-256 |
| --- | --- | --- |
| `Cargo.lock` | `97d50b9ad60ea889b3bb37ff8b50a6e2bba90ada` | `098b922ed192e199028cd3f496ee30d0f7ae76b2a22e44738e3155b89c3079c1` |
| `router/pnpm-lock.yaml` | `d2edd11672dab8c5ba0aa18e6940b504996302ff` | `e10369257410556ba3bbcd210ccd8a8b03974468d958dafca26cdc6297ccc6ac` |
| `scripts/pnpm-lock.yaml` | `495f7d048830e8923bf58bb1f367932765e39d91` | `1720608d1e071c10f99762d318c3bbd02602a9121a1e99f1c479de971882cdf9` |
| `telemetry/pnpm-lock.yaml` | `1ec8b138692ab1b54a6cc083d2acdf870044727b` | `7ded5a40a51ee4ac7c53afba21790734ee301e9dfbec9736b2dbc52f6f5d264f` |
| `vscode/pnpm-lock.yaml` | `16ed0ab89bd9837b186f8d834b811812838908fa` | `3491e27a8c21128697e1e60cb9b9b601446cd9e34a6e5b3adc7a946a1e88b284` |

Read-only ESM resolution probes succeeded from the gate worktree for router
`vitest`, `typescript`, and `ws`; scripts `playwright` and `yaml`; telemetry
`vitest`, `typescript`, and `ws`; and VS Code `typescript` and
`@vscode/vsce`.

## Full-plan enumeration

F0 enumerated, but did not execute, the complete default non-live plan:

```bash
node scripts/verify.mjs --list
```

It passed and expanded selector `verify` to 293 phases, including canonical
`skiff-tests`, seven runtime phases, every other implementation subject, Rust
quality, package type checks, JavaScript syntax, and default checks. F0 did not
run `pnpm verify`, a live selector, a compiler selector, another focused Cargo
test, or any leaf filter.

## Unique post-I3 selector results

R3 production and the N1/E1 test population invalidate the old runtime/source
selector evidence. F0 therefore ran only the assigned replacements, once each,
on the exact input and shared cache:

| Command | Result | Coverage |
| --- | --- | --- |
| `CARGO_TARGET_DIR=/Users/geek/workspace/skiff/build/cargo-target node scripts/verify.mjs --only runtime` | pass, exit 0; all six boundary phases and canonical multi-package Rust phase passed; existing warnings and four expected ignored service-db live tests only | whole runtime subject with integrated R3, E1, and N1 |
| `CARGO_TARGET_DIR=/Users/geek/workspace/skiff/build/cargo-target node scripts/verify.mjs --only skiff-tests` | pass, exit 0; two canonical source entries passed; std 12/12 including the deep tail-recursion source fixture, alias-return-catch-once 7/7, package-service-host 9/9 | fresh source -> compiler -> File IR -> assembly/link -> isolated runtime chain |

Neither selector was retried or rerun. The complete gate remains unexecuted and
reserved for G2 on a later explicit frozen input.

## Result and freeze handoff

The exact input, dependency and lock state, cache identity, capacity, process
exclusivity, A1 blocker closure, completion-owner mapping, I3 evidence, and
both post-I3 selectors are ready for freeze. There is no known F0 blocker.

Required continuation:

```text
this F0_READY_TO_FREEZE document integrated
  -> freeze one exact integration commit/tree
  -> independent A2 verdict on that frozen input
  -> explicit G2 follow-up with the frozen commit/tree
  -> Gate owner aligns this retained worktree exactly to that input
  -> recheck clean/dependencies/locks/cache/capacity/exclusivity
  -> one and only one pnpm verify
```

The branch/worktree, ignored dependencies, and exclusive Cargo target are
intentionally retained. The Gate owner must not run G2 until the main Agent
provides the frozen integration commit/tree, and must not modify, repair, or
rerun the frozen candidate during G2.
