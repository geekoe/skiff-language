# Router Rust Migration Batch 2（M0 + C-net）

日期：2026-08-02
状态：execution batch（主 Agent 调度文档）

## 引用链

权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5，2026-08-01）。
直接父批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-1.md`（已合入本地 main@d1b99360）。
本批次实现权威设计 §2.3 / §5.2 / §5.3 / §5.5 / §6.2(2)，不修改设计语义。

## 批次目标

- M0：收窄 shared Cargo closure，证明 `skiff-router` 不直接/传递依赖宽 `skiff-runtime-model`、
  runtime-host、eval、request execution；把 transport 真正需要的 opaque wire/service-error facts
  下沉到 transport/request-contract 低层 crate；按 §5.3 文件表完成不改变 wire bytes/public API 的
  机械模块拆分；空 Router consumer 可编译共享 envelope/connection identity；稳定 frame-family
  sink registration 契约。
- C-net：在 C0 完成后冻结 final listener 机制：Tokio runtime、HTTP server/upgrade library、
  body streaming type、WS library、graceful shutdown、connection limits；用真实 socket 做
  empty HTTP、HTTP→WS upgrade、connection limit、shutdown probe；机制决策落盘供 PR 0b 直接消费。

退出检查点：两节点合入本地 main（不 push origin），focused 证据通过，一级 worktree 与
已合并临时分支清理完毕。

## DAG 节点

| 节点 | 对应设计条款 | 基线 | 分支 / worktree | 集成目标 |
| --- | --- | --- | --- | --- |
| M0 | §2.3、§5.3、§5.5、§6.1 | main@d1b99360 | `feat/router-rust-m0` / `/Users/geek/workspace/wt-m0` | router_rust_integration_b2 |
| C-net | §5.2、§6.2(2) | main@d1b99360 | `feat/router-rust-c-net` / `/Users/geek/workspace/wt-c-net` | router_rust_integration_b2 |

两节点并行；集成 Agent 串行合入 `integration/router-rust-migration-batch-2`。

## 并行 ownership 边界（写文件声明）

- `router/Cargo.toml`：M0 只改 shared-model 依赖行（transport/request-contract 低层 crate）；
  C-net 只改 net/async dev-dependencies 与 probe 文件（tests/example）。`Cargo.lock` 两者都可能
  再生，允许集成 Agent 机械合并。
- `runtime/transport/src`（含 protocol.rs、lib.rs 及 family 模块）：仅 M0。
- 新增 workspace crate（若 M0 需要）必须在 `scripts/lib/verify-rust-subjects.mjs` 恰好归入一个
  subject（runtime 或 foundation，按 crate 位置决定）：仅 M0。
- `scripts/skiff-instance.mjs`、control plane、config parser、AGENTS.md、scripts README、
  verify selector graph、verify.yml：本批次禁止触碰（除 M0 的 subject 注册）。
- 具体 actor routing projection schema（A0）不属于本批次：M0 只冻结 projection 的 owner/
  依赖方向/reader boundary，不定义具体 schema。
- 任何节点不得操作 stable instance、Mongo、PM2、4004-4007 端口进程；不跑全量 `pnpm verify`。

## 验证 owner

- M0：`cargo test`（transport + request-contract + router contracts，含 golden bytes 不变）；
  `cargo tree -p skiff-router -e normal` 断言不含 `skiff-runtime-model`/runtime-host/eval；
  `node scripts/verify.mjs --only router-rust`；rustfmt/clippy（触碰 crate）。
- C-net：`cargo test --package skiff-router`（含真实 socket probe）、`verify --only router-rust`、
  rustfmt/clippy；机制决策契约文档落盘。
- 集成探针（集成 Agent 唯一 owner）：`verify --only router-rust,router-rust-process-smoke`、
  `cargo tree -p skiff-router -e normal` 负例断言、`cargo test --package skiff-router`。

## 风险与停止条件

- M0 若无法在不引入宽 Runtime execution model 的情况下构建 Router consumer：按设计 §2.3
  停止 handler 方向，先修 crate boundary，返回精确 `cargo tree` 证据。
- C-net 库选择若需要新增计划外公共契约或改变非目标：停止上报，不自行扩展。
- `Cargo.lock`/`router/Cargo.toml` 由两个写入者共享，集成 Agent 只做机械合并；语义冲突
  （如依赖方向违背 M0 gate）停下上报主 Agent。
- 叶子任务发现设计空洞/公共契约变化时返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 流程

每个开发 Agent 默认一次性有界会话：零 worktree 只读预检锚定 main@d1b99360，确认可执行后
创建自己的 worktree（位于 /Users/geek/workspace 下），在第一次 production 修改前形成完整叶子
任务文件（引用本批次文档与权威设计），完成后直接向 router_rust_integration_b2 交接并通知主 Agent。
