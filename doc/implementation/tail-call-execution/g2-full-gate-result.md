# G2 full-gate result

Status: `G2_PASS`

This result records the unique full-gate run for the second tail-call stability
epoch after the host cache was rebuilt. Authority chain:
[`f0-second-ready-to-freeze.md`](./f0-second-ready-to-freeze.md) ->
[`a1-finding-wave-checkpoint.md`](./a1-finding-wave-checkpoint.md) ->
[`parent-checkpoint.md`](./parent-checkpoint.md) ->
[`../../architecture/tail-call-execution.md`](../../architecture/tail-call-execution.md)
and the runtime reference contract.

## Final gated input

- Repository: `/Users/geek/workspace/skiff`
- Final gate commit: `8b4405322c507babb4b04ebdf23a2175378d4631`
- Final gate tree: `62325f8397b4e9f915aba428d0af94ebfea1e953`
- Gate worktree/branch: `/Users/geek/workspace/skiff-tco-final-gate-3` /
  `codex/tco-final-gate-3`
- Exclusive Cargo target: `/Users/geek/workspace/skiff/build/cargo-target`
  (rebuilt cold on 2026-07-31 after the previous cache was intentionally
  removed; ~18 GiB after the gate)
- Toolchain: Node `v25.9.0` (`/opt/homebrew/bin/node`), pnpm `10.29.2`,
  Cargo `1.88.0`, rustc `1.88.0`
- Host capacity after gate: 62 GiB available (86% used)

## Epoch context

The previous F0-2 cache identity was invalidated by an explicit host cleanup,
so this epoch rebuilt the exclusive Cargo target from cold. The frozen
candidate `4af76286` received an independent read-only acceptance PASS, then
the full gate exposed two tooling-phase blockers that were repaired before
the final gate:

| Attempt | Result | Classification | Repair |
| --- | --- | --- | --- |
| 1 | `verify-taxonomy.test.mjs` FAIL (2 assertions) | pre-existing tooling drift, bit-identical on task-before baseline `874ee3a6`; introduced by ancestor `8e9a3c45` | `c9be5973` locks the canonical `checks:compiler-boundaries` phase in the compiler subject |
| 2 | `rust-quality:clippy-baseline` FAIL | branch-caused: new E1 catch/rethrow evidence function exceeds `too_many_lines` without a baseline entry | `8b440532` adds the exact function identity to `scripts/rust-clippy-too-many-lines-baseline.json` |
| 3 | `pnpm verify` PASS, 293 phases | final gate | none |

## Gate command

```bash
cd /Users/geek/workspace/skiff-tco-final-gate-3
CARGO_TARGET_DIR=/Users/geek/workspace/skiff/build/cargo-target \
  PATH=/opt/homebrew/bin:$PATH pnpm verify
```

Result: exit 0; `All selected Skiff verification phases passed.` The canonical
`skiff-tests` real source path, all four Rust implementation subjects, router
and telemetry suites, the complete tooling suite (609 tests), rust-quality
format and clippy baseline, type-checks, JavaScript syntax checks, and all
default checks passed.

## Remaining

`main` contains the TCO integration plus the two tooling repairs
(`4af76286`, `c9be5973`, `8b440532`). The gate worktree and temporary branch
are removed after this result. Push has not been performed and awaits an
explicit user request.
