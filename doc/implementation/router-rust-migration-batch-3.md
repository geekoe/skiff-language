# Router Rust Migration Batch 3（PR 0b + A0 + contract packs）

日期：2026-08-02
状态：execution batch（主 Agent 调度文档）

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，2026-08-01）。
直接父批次：`doc/implementation/router-rust-migration-batch-2.md`（已合入本地 main@1d442366）。
本批次实现权威设计 §2.4 / §5.2 / §5.4 / §5.5 / §6.2(2) / §7 PR0b/E-bootstrap 前置，不修改设计语义。

## 批次目标

- PR 0b：Rust binary 解析冻结后的 strict final Router config（消费与 TS parser 相同的
  golden valid/invalid corpus），使用 C-net 冻结机制启动 public/runtime/control listeners；
  instance build/up 真正构建/安装 `skiff-router` binary，`dev-runtime-paths` 提供 routerBinary，
  process match 支持 TS/Rust spec；`--only router` 只构建 Router；不做业务协议（无 request
  dispatch、无 WS broker、无 activation transaction）。
- A0：冻结 actor routing projection schema/owner/identity generation（stable actor ref、
  method admission/implementation identity、exact deployment binding；不含 source、File IR、
  executable payload），落盘契约文档与 canonical 类型；不实现 A1/A2/A3 consumer。
- Contract pack freeze（bootstrap 链）：C-model-bootstrap-wire + C-model-artifact + C-bootstrap
  的 owner/invariant、typed inputs/outputs、capacity、queue full、timeout/disconnect/
  replacement/shutdown terminal、health fields、fake seam、至少一条真实边界 probe 定义。
- Contract pack freeze（session 链）：C-model-registration + C-session + C-process-lifecycle
  同上，含 byte-exact handshake sequence corpus（accept → router.bootstrap →
  runtime.capabilities → bind → assembly.activation:Register → registered ACK → runtime.health）。
- Contract pack freeze（activation 链）：C-router-activation-state + C-model-activation +
  C-activation-coordinator，含 committed/pending DTO、revision、audit、read/CAS/retry/index/
  driver 契约与 prepare/reject/commit/abort wire 契约。

退出检查点：节点合入本地 main（不 push origin），focused 证据通过，worktree/临时分支清理完毕。

## DAG 节点

| 节点 | 对应设计条款 | 基线 | 分支 / worktree | 集成目标 |
| --- | --- | --- | --- | --- |
| PR 0b | §5.2、§6.2(2) | main@1d442366 | `feat/router-rust-pr0b` / `/Users/geek/workspace/wt-pr0b` | router_rust_integration_b3 |
| A0 | §2.4、§3.4 | main@1d442366 | `feat/router-rust-a0` / `/Users/geek/workspace/wt-a0` | router_rust_integration_b3 |
| contracts-bootstrap | §5.4 bootstrap | main@1d442366 | `feat/router-rust-contracts-bootstrap` / `/Users/geek/workspace/wt-contracts-bootstrap` | router_rust_integration_b3 |
| contracts-session | §5.4 session/registration | main@1d442366 | `feat/router-rust-contracts-session` / `/Users/geek/workspace/wt-contracts-session` | router_rust_integration_b3 |
| contracts-activation | §5.4 activation/persistence | main@1d442366 | `feat/router-rust-contracts-activation` / `/Users/geek/workspace/wt-contracts-activation` | router_rust_integration_b3 |

## 并行 ownership 边界（写文件声明）

- `skiff-router` production 代码（src/main.rs、src/lib.rs、listener 装配、config parser）：
  仅 PR 0b。contract freeze 节点只允许写 contract/corpus 文档与 corpus fixture（可放
  transport/request-contract tests 或独立 fixtures 目录），不得写 skiff-router production。
- `runtime/transport/src`：contracts-session 只碰 registration/handshake/session 相关 corpus
  与模块测试；contracts-bootstrap 只碰 bootstrap-wire/artifact ref 相关 corpus；M0 已完成的
  family 模块结构不得重建。
- `deployment` crate：A0 只加 actor routing projection 类型/模块；contracts-activation 只加
  activation-state DTO/corpus（若放 deployment，需与 A0 模块不重叠）；contracts-bootstrap
  不写 deployment production 代码。
- `artifact-model`：contracts-bootstrap 与 A0 都可能是 owner 候选——先只读预检确认现有类型
  归属；若两节点都必须写同一 crate，按模块划分并在批次文档补充声明，冲突由集成 Agent 上报。
- `scripts/skiff-instance.mjs`：仅 PR 0b（build/up 安装 binary 相关）；其余节点禁止写。
- verify 注册表（`verify-rust-subjects.mjs`）：仅 PR 0b/A0 若新增 workspace crate 时按归属
  注册；contracts 节点若新增 crate 必须先上报。
- AGENTS.md、scripts README、verify selector graph、verify.yml、control plane、config parser
  （TS 侧）：本批次禁止触碰（PR 0b 的 Rust config parser 除外，它消费同一 corpus，不修改 TS
  parser）。
- 任何节点不得操作 stable instance/Mongo/PM2/4004-4007；不跑全量 `pnpm verify`。

## 验证 owner

- PR 0b：`cargo test --package skiff-router`（含真实 socket listener probe）、Rust config
  parser 消费同一 golden corpus（valid 10 / invalid 47 负例）、`verify --only router-rust,
  router-rust-process-smoke`、`check-local-instance.mjs`、隔离 fixture 中 `instance build/up`
  构建并安装 binary（临时 instance，不碰 stable）。
- A0：投影 schema/identity 契约文档 + canonical 类型编译测试 + 反向搜索（consumer 未编码前
  无 File IR 读取）。
- contracts-*：契约文档覆盖 §5.4 pack 全部必填项；corpus fixture 测试通过；反向搜索证明
  无生产 consumer 提前依赖未冻结契约。
- 集成探针：`verify --only router-rust,router-rust-process-smoke`、`cargo test --package
  skiff-router`、contract corpus 测试、`check-local-instance.mjs`。

## 风险与停止条件

- PR 0b 若发现冻结配置契约与 C-net 机制无法组合（例如 corpus 需要新字段），先核对权威设计
  与 C-config 交付，需改公共契约时停止上报，不自行扩 schema。
- contract freeze 节点不得把"待冻结"契约写进 production 代码；只落盘契约文档 + corpus。
- A0 与 contracts-bootstrap 若在 artifact-model/deployment 冲突，集成 Agent 停下上报主 Agent。
- 叶子任务发现设计空洞时返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 流程

每个开发 Agent 默认一次性有界会话：零 worktree 只读预检锚定 main@1d442366，确认可执行后创建
自己的 worktree（位于 /Users/geek/workspace 下），在第一次 production 修改前形成完整叶子任务
文件（引用本批次文档与权威设计），完成后直接向 router_rust_integration_b3 交接并通知主 Agent。
