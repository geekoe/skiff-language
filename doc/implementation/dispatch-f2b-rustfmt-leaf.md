# Leaf Task: F2b workspace canonical rustfmt 漂移修复

## 引用链

- Gate 命令：`scripts/lib/verify-plan.mjs` 的 `rust-quality` scope 执行
  `cargo fmt --all -- --check`（`scripts/verify.mjs` 经
  `.github/workflows/verify.yml` 的 `Quality and Checks` job 运行）。
- Canonical 工具链：CI 使用 `rustup toolchain install stable --profile minimal
  --component rustfmt --component clippy` + `rustup default stable`；仓库无
  `rust-toolchain*` 固定文件，无 `rustfmt.toml` / `.rustfmt.toml`。本机 stable
  rustc 1.88.0 / rustfmt 1.8.0-stable，与 gate canonical 一致。
- baseline：`3f2d7d57b399f16b114b678310e342b55063589d`
  （`dispatch-e-integration` HEAD，`git rev-parse` 确认）。
- worktree：`/Users/geek/workspace/skiff-f2b-fmt`，branch `rustfmt-fix`。
- 集成 Agent：`/root/dispatch_e_integration`；主 Agent：`/root`。本任务不 merge、
  不 push、不写共享集成分支；共享主 worktree 只读。

## 任务合同摘要

1. 预检 canonical rustfmt 命令与工具链，记录 `cargo fmt --all -- --check`
   基线 diff 规模。
2. 对整个 workspace 执行 `cargo fmt --all`，一次性机械 commit（不改语义；
   格式化工具行为导致语义风险时停止并上报）。
3. 验证：fmt check 零 diff、`git diff --check` 干净、`cargo check --workspace`
   通过；写本叶子文档记录命令、diff 规模与写集统计。
4. 提交后报告 `/root/dispatch_e_integration` 并通知 `/root`。

## 预检结论（只读，锚定 3f2d7d57）

- canonical 命令确认为 `cargo fmt --all -- --check`；本机 `rustfmt --version`
  输出 `1.8.0-stable`，与 gate 指定 stable rustfmt 1.8.0 一致。
- 基线（格式化前）：`cargo fmt --all -- --check` exit 1；543 个 diff 块、
  117 个文件（全部 `.rs`）、日志 +/- 行 +2043/−1709。
- 基线 diff 块按 crate 域分布：`router/tests` 136、`router/src` 130、
  `runtime/transport` 129、`runtime/eval` 35、`runtime/host` 25、
  `compiler/lowering` 13、`runtime/boundary` 11、`task-control/tests` 10、
  `runtime/tests` 9、`compiler/source` 9、`task-control/src` 8、
  `compiler/core` 7、`syntax/src` 6，其余（capability-context、
  test-runner、request、linker、native、linked-type-plan、driver、
  deployment）各 1–4。

## 执行记录

命令（全部在 worktree `/Users/geek/workspace/skiff-f2b-fmt` 执行，仅
`cargo fmt` 自动改写，未手动修改任何代码）：

```bash
cargo fmt --all -- --check   # 基线：exit 1
cargo fmt --all             # 一次性格式化
cargo fmt --all -- --check   # 验证：exit 0，零 diff
git diff --check            # 干净
cargo check --workspace     # exit 0
```

写集统计（commit `1eedb244`，父 `3f2d7d57`）：

- `117 files changed, 2024 insertions(+), 1686 deletions(-)`，全部为 `.rs`
  源码文件，无其他扩展名改动。
- 文件数按 crate 域分布：`router/src` 22、`runtime/transport` 15、
  `runtime/eval` 13、`runtime/host` 12、`router/tests` 11、
  `compiler/source` 7、`runtime/boundary` 6、`compiler/lowering` 5、
  `task-control/src` 4、`syntax/src` 4、`compiler/core` 4、
  `runtime/tests` 3、`task-control/tests` 2、`runtime/linker` 2，
  `test-runner/src`、`runtime/request`、`runtime/native`、
  `runtime/linked-type-plan`、`runtime/driver`、
  `runtime/capability-context`、`deployment/src` 各 1。
- 提交仅含格式改动（`git show --name-only` 确认 117/117 为 `.rs`；
  `git diff --check` 无空白错误）。

## 自验收矩阵

| 验证项 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0，零 diff |
| `git diff --check` | 干净 |
| `cargo check --workspace` | exit 0；仅既有 dead-code / unused warnings（如 skiff-runtime-host 14、skiff-router 3），无 error；worktree 首次全量 check 约 16.46s，二次缓存 0.15s |
| 代码语义 | 未手动改代码；格式化后 workspace 编译通过 |

## 禁止与合规

- 未 push；未写共享集成分支 `dispatch-e-integration`。
- 未动共享主 worktree（`/Users/geek/workspace/skiff` 只读，无改动）。
- 未手动改任何代码；仅 `cargo fmt --all` 自动改写。
- 未改 `doc/reference/`、`doc/architecture/` 与 `doc/implementation/` 既有文件
  （本叶子文件为新增）。
- 未跑完整 gate；仅执行任务要求的聚焦验证。

## 交接

分支 `rustfmt-fix`（worktree `/Users/geek/workspace/skiff-f2b-fmt`）含两个
commit：`1eedb244`（机械格式 commit）与本叶子文档 commit。已报告集成 Agent
`/root/dispatch_e_integration` 并通知主 Agent `/root`。
