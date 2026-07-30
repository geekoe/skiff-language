# P5-F445G-R4 Composite integration acceptance result

状态：`PASS / NO_COMPOSITION_REGRESSION`。

I3、Router File IR v9 consumer 与 admission correction 的合成树通过任务指定矩阵。没有出现
composition regression；父 result 已记录的 compiler/Router full-suite fixture debt 不在本次聚焦
矩阵中，也没有被本结果重新归类为通过。

## 1. 输入与只读执行环境

| 项 | 值 |
| --- | --- |
| 固定 production input | `d5812c27` |
| 验收 worktree HEAD | `71cefcb5` |
| 验收 Cargo target | `/Users/geek/workspace/skiff-p5-f445g-r4-composite-acceptance/build/cargo-target` |
| 本地依赖复用 worktree HEAD | `63adcb38` |

Rust 矩阵全部在任务 worktree 运行，并严格使用上述专属 Cargo target。

任务 worktree 没有 `node_modules`，三条 Router 命令的首次探针分别以
`vitest: command not found`、`vitest: command not found` 和 `tsc: command not found`
结束。这是 fresh worktree 的本地工具前置条件缺失，不是测试断言失败。没有安装依赖或访问网络。

随后在已有本地依赖的 `/Users/geek/workspace/skiff-phase-05-integration` 只读重跑三条 Router
命令和反搜。该 worktree 相对 `d5812c27` 仅新增本任务与后继任务文档；以下命令确认 Router、
Rust production、test、golden 与 lockfile 均无差异：

```text
git diff --name-only d5812c27..63adcb38
  P5-F445G-R4-composite-integration-acceptance.md
  P5-F445H-eval-concurrency-owner-preflight.md

git diff --quiet d5812c27..63adcb38 -- \
  router Cargo.toml Cargo.lock artifact-model compiler runtime
  exit 0
```

因此 Router 重跑消费的是与固定 input 完全相同的 production/test 内容，仅复用已安装且未修改的
本地依赖。

## 2. 验收矩阵

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-artifact-model timeout_execution -- --nocapture` | PASS：3 passed，0 failed，176 filtered |
| `cargo test -p skiff-compiler --test timeout_artifact_lowering -- --nocapture` | PASS：4 passed，0 failed |
| official std deterministic authoring exact test | PASS：选中项 1 passed，0 failed；其它 target 均为 0 selected |
| canonical builtin spelling focused test | PASS：1 passed，0 failed，8 filtered |
| `cargo test -p skiff-compiler --test package_interface_identity -- --nocapture` | PASS：4 passed，0 failed |
| `cargo test -p skiff-runtime-linked-program --test timeout_execution -- --nocapture` | PASS：1 passed，0 failed |
| `cargo test -p skiff-runtime-linker timeout_execution -- --nocapture` | PASS：7 passed，0 failed，51 filtered |
| `cargo test -p skiff-runtime-linker --no-fail-fast` | PASS：58 unit passed，0 failed；0 doc-tests |
| `cargo check -p skiff-compiler --locked` | PASS；27 个既有 compiler-source warning，无 error |
| compiler-generated manifest compatibility | PASS：1 file，1 test |
| dynamic build-id parity | PASS：1 file，4 tests |
| `pnpm --dir router type-check` | PASS：`tsc --noEmit` exit 0 |
| `cargo fmt --check` | PASS：exit 0 |
| `git diff --check` | PASS：exit 0 |

所有被选测试合计为 84 passed、0 failed：

- Rust：79 passed；
- Router：5 passed；

这里按每条命令的真实执行计数相加；linker focused 的 7 项也包含在 linker full 的 58 项内，
因此该合计是执行次数，不是去重后的测试用例数。

## 3. Active consumer 反搜

```text
rg -n 'skiff-file-ir-v8' \
  router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts \
  router/tests/compilerGeneratedManifestCompatibility.test.ts
```

结果为 0 match；`rg` 按零匹配语义返回 exit 1。当前 filesystem loader 与
compiler-generated compatibility consumer 均不再接受或断言 v8。

## 4. 结论与边界

- I3 timeout artifact/lowering/link identity、admission correction 与 Router v9 reader 的真实组合
  全部通过。
- 没有 composition regression。
- 父结果记录的 compiler 七组 fixture debt 与 Router full-suite actor-spawn 文本 debt 不在本任务
  指定矩阵中；本次没有触发新的失败，也不宣称这些 inherited debt 已关闭。
- 除本 result 外，没有修改 production、test、golden 或 lockfile。
- 没有派子 Agent，没有 merge、rebase、push、stable、live、网络访问或依赖安装。
