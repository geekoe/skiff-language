# P5-F445G-R4 Composite integration acceptance

状态：Ready。只读验收 I3 + Router child + review correction 的合成树。

## 直接父节点

- `P5-F445G-timeout-artifact-lowering-link-checkpoint-result.md`
- `P5-F445G-R1-router-file-ir-v9-admission-result.md`
- `P5-F445G-R3-timeout-ir-admission-correction-result.md`

固定输入：

`/Users/geek/workspace/skiff-phase-05-integration` @ `d5812c27`

## 验收目标

不修改 production/test/golden，验证 cherry-pick 与两处 std build golden conflict resolution 后的
真实组合：

```bash
cargo test -p skiff-artifact-model timeout_execution -- --nocapture
cargo test -p skiff-compiler --test timeout_artifact_lowering -- --nocapture
cargo test -p skiff-compiler \
  authoring::package_publication::tests::official_std_authoring_and_record_writer_are_fixed_and_deterministic \
  -- --exact --nocapture
cargo test -p skiff-compiler --test builtin_canonical_spelling \
  declared_source_aliases_emit_only_canonical_file_ir_builtin_names -- --nocapture
cargo test -p skiff-compiler --test package_interface_identity -- --nocapture
cargo test -p skiff-runtime-linked-program --test timeout_execution -- --nocapture
cargo test -p skiff-runtime-linker timeout_execution -- --nocapture
cargo test -p skiff-runtime-linker --no-fail-fast
cargo check -p skiff-compiler --locked
pnpm --dir router exec vitest run tests/compilerGeneratedManifestCompatibility.test.ts
pnpm --dir router exec vitest run tests/dynamic-build-id-parity.test.ts
pnpm --dir router type-check
cargo fmt --check
git diff --check
```

所有 Cargo 命令使用：

`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445g-r4-composite-acceptance/build/cargo-target`

并反搜 active current consumer：

```bash
rg -n 'skiff-file-ir-v8' \
  router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts \
  router/tests/compilerGeneratedManifestCompatibility.test.ts
```

必须零匹配。

## 输出

只新增并提交：

`P5-F445G-R4-composite-integration-acceptance-result.md`

给出 `PASS` 或 `FAIL`，记录每条命令的精确计数；若失败，区分 composition regression 与父 result
已记录的 inherited fixture debt。不得修复、修改其它文件、派子 Agent、merge/rebase/push、
stable/live/network。最终 clean。

## worktree

`/Users/geek/workspace/skiff-p5-f445g-r4-composite-acceptance`

branch：

`codex/p5-f445g-r4-composite-acceptance`

base：`d5812c27`，再 cherry-pick 本任务文档。
