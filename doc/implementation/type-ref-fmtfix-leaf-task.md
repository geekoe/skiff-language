# Leaf Task: rustfmt service-db tests 机械修复

## 引用链

- 权威设计：`doc/implementation/compiler-type-ref-unification-plan.md`（v3）。本任务不改变设计
  语义，是恢复 `main` 的 `rust-quality:format` 门禁的机械格式化收尾。
- 直接父节点：主 Agent（/root）任务信封 `skiff_dev_fmtfix`（本任务），集成 Agent
  `skiff_integration_phase1`。
- 流程：`/Users/geek/workspace/multi-agent-development.md`（开发 Agent 角色、零 worktree 预检、
  叶子执行合同、自验收、交接给 `skiff_integration_phase1`）。

## Baseline

- repo：`/Users/geek/workspace/skiff`
- baseline：`main` @ `3ce3fb3e5a9b09be2a5d28817ac517abe29cafaf`（当前 HEAD，工作区干净）
- worktree：`/Users/geek/workspace/skiff-fmtfix`，branch `fix/rustfmt-service-db-tests`

## 零 worktree 只读预检（已执行）

`cargo fmt --all -- --check`（在 baseline HEAD 上，只读）输出唯一的 Diff：
`runtime/service-db/src/tests.rs:628`，即 `assert_eq!(exact.database_name,
"example~com~~service");` 多行展开区域，来自既有提交 `1c7d5795`
（`fix(service-db): restore readable service database names`），与 TypeRef 阶段无关。
其余文件全部符合 rustfmt。

## 写集

- `runtime/service-db/src/tests.rs`（唯一格式化变化：`assert_eq!` 折叠为单行，
  `+1 -4`，无语义变化）
- `doc/implementation/type-ref-fmtfix-leaf-task.md`（本叶子执行合同）

## 执行决策

1. 只运行 `cargo fmt --all`，接受 rustfmt 对 `runtime/service-db/src/tests.rs:628` 区域的机械重排。
2. 不得改动任何其他文件；`git diff --stat` 与 `git diff` 确认仅上述文件变化且无语义变化。
3. 不运行测试（纯格式化，任务信封明确豁免）。
4. 不 push、不写 main、不承接其他任务。

## 验证矩阵

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| 唯一未格式化文件是 service-db tests.rs | `cargo fmt --all -- --check` 只报 `runtime/service-db/src/tests.rs:628`（baseline 预检） | `git status --short` 仅 `runtime/service-db/src/tests.rs` | 提交后 `cargo fmt --all -- --check` 退出码 0 |
| 格式化无语义变化 | `git diff` 仅 `assert_eq!` 多行→单行（`+1 -4`） | `git diff --stat` 1 file changed | 无需测试（纯格式化） |

## 交接

- branch：`fix/rustfmt-service-db-tests`
- worktree：`/Users/geek/workspace/skiff-fmtfix`
- commit：`style: rustfmt service-db tests`（提交后记录 commit/tree）
- 写集：见上；证据：见验证矩阵
- 交接对象：集成 Agent `skiff_integration_phase1`；同时通知主 Agent（/root）
