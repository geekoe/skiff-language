# Router Rust Migration C-net Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界开发会话）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5，2026-08-01），
  重点 §5.2（C-net 与 PR 0b）、§6.2(2)（tooling 持续推进）、§7 PR0b（C-net 为 PR0b 前置）。
- 直接父节点：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-2.md`（M0 + C-net 批次调度）。
- 本叶子执行 contract 决策落盘：`doc/implementation/router-rust-migration/contracts/router-rust-migration-c-net-contract.md`。

冲突时以权威设计为准；本文件只补充执行信息，不改变设计语义。

## 任务目标

在 `skiff/` 仓库内冻结 final listener 机制（供 PR 0b 直接消费），并用真实 socket 验证：

1. 落盘决策文档（`router-rust-migration-c-net-contract.md`）：Tokio runtime、HTTP
   server/upgrade library、body streaming type、WS library、graceful shutdown、
   connection limits；给出最小充分方案、反事实说明与被拒绝候选。
2. 在 `skiff-router` crate 内用真实 socket 做 probe（`router/tests/`）：
   empty HTTP request/response、empty HTTP→WebSocket upgrade、connection limit、
   graceful shutdown（drain + deadline 强制关闭）。
3. 只冻结 mechanism，不冻结 HTTP 业务 ports/paths，不实现业务协议/控制端点。

## 基线与环境

- 仓库：`/Users/geek/workspace/skiff`；精确 baseline：`main@d1b99360`
  （Batch 1 已合入；`git rev-parse d1b99360` 已确认）。
- 分支 / worktree：`feat/router-rust-c-net` / `/Users/geek/workspace/wt-c-net`。
- `CARGO_TARGET_DIR=/Users/geek/workspace/wt-c-net/target`（不与其他 worktree 共享）。
- 集成 Agent：`/root/router_rust_integration_b2`；完成后直接交接并通知 root。

## 写入边界

可写：

- `router/Cargo.toml`：仅 `[dev-dependencies]` 中 net/async 与 probe 所需依赖；
  不触碰 shared-model 依赖行（M0 独占）。
- `router/tests/net_probe.rs`（probe 文件；不新增生产 listener 代码）。
- `Cargo.lock`：仅 net 依赖部分（允许机械合并）。
- `doc/implementation/router-rust-migration/execution/router-rust-migration-c-net-leaf.md`、
  `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-net-contract.md`。

禁止：

- `router/Cargo.toml` 的 shared-model 依赖行（M0）、`runtime/transport/`、
  `scripts/lib/verify-rust-subjects.mjs`、`scripts/skiff-instance.mjs`、
  control plane、config parser、AGENTS.md、scripts README、verify selector graph、
  verify.yml、任何公共契约/注册表改动。
- 操作 stable instance、Mongo、PM2、4004-4007 端口进程；probe 不得依赖 stable
  instance 或外部网络；不跑全量 `pnpm verify`。

## 机制决策摘要（完整版见 contract 文档）

- Tokio runtime：tokio 1.x multi-thread runtime（workspace lock 为 1.52.3；
  runtime/host 已使用同族 feature）。
- HTTP server/upgrade：hyper 1（`hyper::server::conn::http1::Builder`，
  features `http1` + `server`）+ `serve_connection(...).with_upgrades()`；hyper-util
  0.1 仅提供 `TokioIo` 适配（feature `tokio`）。probe 实测：不带
  `.with_upgrades()` 时 upgrade 以 `ManualUpgrade` 失败。不启用 HTTP/2
  （无设计需求，TS Router 是 HTTP/1.1；h2 不在 baseline lock 中）。
- Body streaming type：`http_body` trait（`Data = Bytes`）；service 响应类型
  `http_body_util::Full<Bytes>`（empty/fixed body），boxed 流式边界类型
  `BoxBody<Bytes, hyper::Error>` 供 PR 0b 使用；request body 为 `hyper::body::Incoming`。
- WS library：tokio-tungstenite 0.26（runtime/host 已用）；模式为 hyper upgrade
  → 101 响应（`tungstenite::handshake::derive_accept_key` 计算
  `Sec-WebSocket-Accept`）→ `WebSocketStream::from_raw_socket(Role::Server)`。
- Graceful shutdown：accept loop 收到 watch 信号后停止 accept；每个连接任务
  select 同一 watch，调用 hyper `UpgradeableConnection::graceful_shutdown()` 并等待
  自然结束；超过 deadline 后 abort 剩余连接任务。升级后的 WS 连接脱离 hyper
  连接，由 supervisor 独立跟踪（JoinSet）并在 deadline 后 abort。完整
  C-process-lifecycle 停机顺序属后续 lane。
- Connection limits：accept 时 `tokio::sync::Semaphore::try_acquire_owned`；
  超限连接收到 `503` + `Connection: close` 后关闭；permit 持有到连接结束。

## 自验收命令（worktree 内，`CARGO_TARGET_DIR` 指向本 worktree target）

- `cargo test --package skiff-router`（含真实 socket probe）。
- `node scripts/verify.mjs --only router-rust`（Rust subject 的 contracts leaf）。
- `cargo fmt --check`（router crate 范围）与 `cargo clippy --package skiff-router
  --all-targets`（router crate）。
- `git diff d1b99360 -- Cargo.lock` 检查无意外大变更（应只新增 skiff-router 的
  dev-dependency 列表，无新 crate 版本）。

## 交接物

- 实现 commit/tree、worktree 路径、分支名、实际写集、自验收矩阵；
  直接报告 `/root/router_rust_integration_b2` 并通知 root。
