# Router Rust Migration Batch 6（E-bootstrap gate + routing/dispatch/http/spawn-cut）

日期：2026-08-02
状态：execution batch（主 Agent 调度文档）

## 引用链

权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，2026-08-01）。
直接父批次：`doc/implementation/router-rust-migration-batch-5.md`（已合入并 push 到
origin/main@8cabf352）。本批次实现权威设计 §5.4 W-routing-query/W-dispatch/W-http、
§5.3 W-model-request/M-request、§7 E-bootstrap、§5.4 H-spawn-parent-cut 前置，
不修改设计语义。

## 批次目标

- E-bootstrap gate：生产装配（repository → `CommittedActivationBootstrapReader` → strict
  loader → `ActiveRoutingEpochStore` → `SessionLayer` epoch source，接线进 run_router）+
  `router-live:bootstrap` managed harness（真实 compiler artifact → committed reader →
  initial epoch；missing/malformed/identity mismatch/pending 全 fail closed）+ verify live
  registry 条目 + `.github/workflows/router-rust-integration.yml`（cheap change classifier，
  相关 PR 跑 managed job，非相关显式成功）。这是首个 live slice，CI workflow 本批次建立。
- W-routing-query：stateless exact candidate projection（captured `RoutingEpoch` + directory
  exact tuple/registration revision/cancellation → `RegisteredSessionLease`），dispatch 与
  activation 共用同一 sequence corpus（C-routing-query）。
- W-model-request + M-request：request wire DTO/codec/corpus（runtime_assembly_request 现有
  形态）+ Router/Runtime consumer gate；golden 按 contracts-request corpus。
- W-dispatch：epoch capture → candidate query → admission permit（reserve/revalidate/
  释放）→ enqueue → terminal；`RequestDispatcher` pending/terminal 与 function-spawn
  correlation（actor-method spawn 归 actor lane）；消费 C-dispatch corpus。
- W-http：HTTP socket、trusted selector、service-scoped ingress、body/stream/CORS 映射到
  dispatcher port；首个真实边界为 real HTTP → fake dispatcher（不接 run_router，E-http
  再接真实链路）。
- H-spawn-parent-cut：current TS Router 与 Rust Runtime 硬切 spawn `callerKind` 新 wire，
  删除旧 shape、无兼容 reader；C-spawn 由此解锁。

退出检查点：节点合入本地 main，探针通过，push origin/main，worktree/临时分支清理完毕。

## DAG 节点

| 节点 | 设计条款 | 基线 | 分支 / worktree |
| --- | --- | --- | --- |
| E-bootstrap gate | §7 E-bootstrap、§8 router-live:bootstrap、§8 CI | main@8cabf352 | `feat/router-rust-e-bootstrap-gate` / `wt-e-bootstrap-gate` |
| W-routing-query | §5.4 C-routing-query、§7 E-dispatch 前置 | main@8cabf352 | `feat/router-rust-w-routing-query` / `wt-w-routing-query` |
| W-model-request | §5.3 W-model-request/M-request | main@8cabf352 | `feat/router-rust-w-model-request` / `wt-w-model-request` |
| W-dispatch | §5.4 C-dispatch、§7 E-dispatch | main@8cabf352 | `feat/router-rust-w-dispatch` / `wt-w-dispatch` |
| W-http | §5.4 W-http、§7 E-http 前置 | main@8cabf352 | `feat/router-rust-w-http` / `wt-w-http` |
| H-spawn-parent-cut | §5.3 C-spawn 解锁、§5.4 H-spawn-parent-cut | main@8cabf352 | `feat/router-rust-h-spawn-parent-cut` / `wt-h-spawn-parent-cut` |

## 并行 ownership 边界（写文件声明）

- run_router/main.rs/listener.rs 生产装配：仅 E-bootstrap gate。W-http 只写
  `router/src/http/` 模块与 `router/tests/http_*`，不接 run_router。
- `router/src/` 新模块：routing-query 归 W-routing-query（`src/routing/query.rs` 或既有
  candidate 位置由预检决定）、dispatch 归 W-dispatch（`src/dispatch/`）、http 归 W-http
  （`src/http/`）、bootstrap 生产装配归 E-bootstrap gate（`src/bootstrap/` 已有模块）。
- `runtime/transport/src`：request 模块仅 W-model-request。
- router TS 与 runtime crate driver：仅 H-spawn-parent-cut。
- `scripts/lib/verify-live-registry.mjs`、`.github/workflows/`：仅 E-bootstrap gate。
- AGENTS.md、scripts README、verify selector graph、`scripts/skiff-instance.mjs`、
  deployment：本批次禁止触碰。
- 任何节点不得操作 stable instance/Mongo/PM2/4004-4007；不跑全量 `pnpm verify`。

## 验证 owner

- E-bootstrap gate：`router-live:bootstrap` harness 通过（真实 compiler artifact/committed
  reader/initial epoch + fail-closed 负例）、`verify --only router-rust,router-rust-process-smoke`
  + `--list` 含新 live 条目、CI workflow YAML 可解析、`gh run list` 观测 Batch 5 Verify 状态
  并报告（不阻塞）。
- W-routing-query / W-dispatch / W-http / W-model-request：各模块 cargo test + corpus 测试 +
  `verify --only router-rust`；W-http 真实 socket HTTP→fake dispatcher 探针。
- H-spawn-parent-cut：router TS tests + runtime tests 全绿；rg 负例：旧 spawn shape
  （无 callerKind 的 outbound）在 production 零命中。
- 集成探针：`verify --only router-rust,router-rust-process-smoke`、`cargo test -p skiff-router`、
  `cargo test -p skiff-runtime-transport -p skiff-deployment`、`cargo test -p runtime`、
  `check-local-instance.mjs`、CI workflow YAML 解析。

## 风险与停止条件

- E-bootstrap gate 若发现 repository read port 与 W-bootstrap 契约不一致，先核对
  contracts-bootstrap/W-activation-state 交付，需改公共契约时停止上报。
- W-http 不接 run_router（避免与 E-bootstrap 装配冲突）；需要接线时上报集成 Agent。
- H-spawn-parent-cut 是生产 wire 硬切：先过共享 corpus 再改 production，不写兼容 reader。
- 叶子任务发现设计空洞返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 流程

每个开发 Agent 默认一次性有界会话：零 worktree 只读预检锚定 main@8cabf352，确认可执行后创建
自己的 worktree（位于 /Users/geek/workspace 下），在第一次 production 修改前形成完整叶子任务
文件（引用本批次文档与权威设计），完成后直接向 router_rust_integration_b6 交接并通知主 Agent。
集成 Agent 在全部合入、探针通过后把 main push 到 origin/main（已授权）。
