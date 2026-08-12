# AUD0: exact baseline receipt

> Status: completed
> Baseline: `7915d634 docs: make phase execution mapping rolling`

## 1. Repo identity

- Repository: Skiff language and runtime, workspace root `/Users/geek/workspace/skiff`.
- Git worktree: `/Users/geek/workspace/skiff` on `main`, ahead of `origin/main` by
  180 commits at audit time.
- Main checkout was clean before the Phase 0 leaf was created.
- Phase leaf: `/Users/geek/workspace/skiff-phase-0` on `bytecode-vm-phase-0`.

## 2. Topology

- Rust workspace: `Cargo.toml`, including `compiler/`, `runtime/`, `test-runner/`,
  `router/`, artifact/identity/deployment crates, and scheduler/request crates.
- Compiler CLI: `compiler/driver/bin/skiff-compiler.rs`; top-level scripts:
  `scripts/skiff.mjs`, `scripts/verify.mjs`, `scripts/run-skiff-tests.mjs`.
- Production request path lives in `runtime/request`; VM core in `runtime/vm`;
  loader/linker/verifier/image in `runtime/loader`, `runtime/linker`,
  `runtime/bytecode-verifier`, `runtime/deployment-image`.
- Canonical bytecode architecture: `doc/architecture/bytecode-vm.md`.
- Review input: `doc/architecture/bytecode-vm-architecture-review.md`.
- Phase plan: `doc/implementation/bytecode-vm-convergence/`.

## 3. Clean baseline constraints

- No dirty main checkout was present at `git status --short --branch`.
- Phase 0 audit reads and writes are separated: audits recorded evidence in the
  leaf; production code is not changed by the audits.
- Build artifacts and run dirs are ignored by the repository and were not
  modified for this audit.
