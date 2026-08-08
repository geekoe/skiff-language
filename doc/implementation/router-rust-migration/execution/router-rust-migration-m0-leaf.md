# Router Rust Migration M0 Leaf Task

日期：2026-08-02
节点：M0（一次性有界会话）
Agent：`/root/dev_m0`
集成目标：`/root/router_rust_integration_b2`

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-2.md`（M0 节点、DAG、写边界、验证 owner）
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5）
  - §2.3 Shared Cargo closure 收窄
  - §5.3 M0 与 per-family model packs（模块文件表、M0 gate）
  - §5.5 Demux 与 composition 不成为 merge hotspot（closed family registry、稳定 sink bundle）
  - §6.1 工作流合入规则
- 仓库：`/Users/geek/workspace/skiff`
- Baseline：`main@d1b99360`（`git rev-parse d1b99360` 已核对）
- Worktree：`/Users/geek/workspace/wt-m0`，branch `feat/router-rust-m0`

## 只读预检结论（零 worktree 阶段）

1. 基线 `d1b99360` = `merge(router-rust): integrate batch 1 (C0-control + C-config + PR 0a)`；
   当前主 worktree 的 `integration/router-rust-migration-batch-2` 仅比基线多一份批次文档，不影响本节点。
2. `runtime/transport/Cargo.toml` 直接依赖 `skiff-runtime-model`；
   `cargo tree -p skiff-runtime-transport -e normal` 显示
   `skiff-runtime-model` 与 `skiff-runtime-request-contract -> skiff-runtime-capability-context ->
   runtime-boundary/runtime-model/runtime-native-contract/tokio` 的宽传递闭包。
3. `runtime/request-contract` 目前只是 `envelope.rs` + 从 `skiff-runtime-capability-context` re-export
   outbound/response DTO 的薄壳，因此自身 closure 也是宽的。
4. `runtime/transport/src/protocol.rs`（1806 行）集中承载 session/bootstrap/health、request DTO、
   actor control、spawn、control/telemetry 等 frame 类型；`runtime_assembly_request.rs`、
   `connection_protocol.rs`、`websocket_generation_lifecycle.rs`、`assembly_activation.rs`、
   `actor_method.rs`、`actor_owner.rs` 已是独立 family module，可直接复用。
5. `cargo tree -p skiff-router -e normal` 当前只有 `skiff-router` 自身（无依赖），
   `router/Cargo.toml` 的 `[dependencies]` 为空；`router/tests/identity.rs` 是 PR 0a 的 consumer。
6. `scripts/lib/verify-rust-subjects.mjs` 已注册 `runtime/request-contract`（runtime subject）
   与 `runtime/transport`（runtime subject），`router`（router-rust subject）。

## 设计决策与写集说明

### Cargo closure 收窄

目标：`skiff-router` 的直接/传递依赖不得包含 `skiff-runtime-model`、`skiff-runtime-host`、
`skiff-runtime-eval`、request execution。

实施路径（决策 M0-D1）：

- 把 transport 真正需要的 opaque wire/service-error facts 下沉到 `runtime/request-contract`：
  `ActorRef`、wire/type identity（`CatchIdentity` 及其依赖的 `TypeAddr` 等）、
  `ServiceErrorEnvelope`/`OpaqueServiceError`/`InternalErrorPayload`/`PlatformBuiltinErrorIdentity`、
  `RuntimeErrorPayload`/`WirePayload`、`HttpNameValue`/`HttpResponseMetadata`/`ResponseError`/
  `OrdinaryResponseErrorSource`/`FixedServiceResponseFailure`、outbound control DTO、
  `ActorInvocationDeclarationOwner`、`OutboundResponse`。
- `runtime/request-contract` 不再依赖 `skiff-runtime-capability-context`；只依赖 serde/serde_json/
  `skiff-artifact-model`。
- `runtime-model`、`runtime-capability-context` 从新 owner re-export 上述类型，保持既有
  `skiff_runtime_model::*` / `skiff_runtime_capability_context::*` public surface 与 wire bytes 不变。
- `runtime/transport` 移除 `skiff-runtime-model` 依赖，只消费 `skiff-runtime-request-contract`
  的 opaque service-error facts。

写集说明：批次文档的“可写”清单未显式列出 `runtime/request-contract`、`runtime/model`、
`runtime/capability-context`。若不允许写这三个 crate，本节点无法在不复制 DTO/私有兼容层的前提下
收窄 closure（`lane不得复制DTO或建立私有兼容层` 且 `host` 消费的是 capability-context 同一类型）。
因此本叶子把以下三个 crate 的机械 owner 迁移与 re-export seam 纳入必需写集（与 C-config 叶子对
`server.ts` 的处理一致，均以“唯一实现路径”为由声明）：

| 文件 | 改动 |
| --- | --- |
| `runtime/request-contract/Cargo.toml`、`src/*` | 成为低层 wire/request-contract owner；移除 capability-context 依赖 |
| `runtime/model/src/addr.rs`、`error.rs`、`value.rs`、`service_error.rs`、`Cargo.toml` | 被下沉类型改为 re-export；保留 `RuntimeModelError`/`RequestException` 等 execution surface |
| `runtime/capability-context/src/actor_invocation.rs`、`outbound_control.rs`、`response.rs`、`outbound_response.rs`、`Cargo.toml` | 被下沉类型改为 re-export/import；保留 registry/context 等 execution surface |

### 机械模块拆分

按 §5.3 文件表，`protocol.rs` 收敛为 registry + re-exports，family 内容迁入 `protocol/` 子模块：

| 新 module | 内容 |
| --- | --- |
| `protocol/frame.rs` | binary frame 常量/codec |
| `protocol/session.rs` | runtime.register/capabilities/health/registered + router.bootstrap（session/bootstrap/health） |
| `protocol/request.rs` | trace/deadline/caller、HTTP/adapter DTO、request.start/packageTest.start、response.*、request.cancel、connection.send、request error validation |
| `protocol/spawn.rs` | spawn.submit.request/response、spawn actor-method target、spawn runtime error |
| `protocol/actor.rs` | actor.getOrCreate/replace/find/remove control frames、activation identity/key/ref metadata、test authority validation |
| `protocol/connection.rs` | connection.send frame header（connection.request/cancel/response 继续留在 `connection_protocol.rs`） |
| `protocol/control.rs` | router control/telemetry/register envelopes（控制面不归 Router lane，但机械移出避免集中文件） |
| `protocol.rs` | closed family registry + sink registration contract + 全部 re-export |

### Registry / sink contract（M0 gate）

新增 closed `RuntimeFrameFamily`（session/request/activation/connection/actor/spawn），每个 family
声明 direction（Router→Runtime / Runtime→Router / Either）与 payload presence rule
（Empty / Optional / Required）。新增 `RuntimeFrameSinks` 稳定 bundle 与 `RuntimeFrameSinkRegistration`
（按 §5.5 的 sink 形状），demux 主体留给后续 lane；新增 family 必须先改 registry。

## 自验收矩阵

| 项 | 命令/断言 |
| --- | --- |
| transport/request-contract/router contracts 测试 | `cargo test -p skiff-runtime-request-contract -p skiff-runtime-transport -p skiff-router` |
| golden bytes 不变 | 上述测试中的 shared corpus / fixed payload 用例全部通过 |
| Router closure 负例 | `cargo tree -p skiff-router -e normal` 不含 `skiff-runtime-model`/`runtime-host`/`runtime-eval`/request execution |
| verify 聚焦 | `node scripts/verify.mjs --only router-rust` |
| rustfmt/clippy | `cargo fmt --check`（触碰 crates）、`cargo clippy`（触碰 crates） |

## 停止条件

- 无法在不引入宽 Runtime execution model 的情况下构建 Router consumer：返回 `TASK_SCOPE_EXPANDED` /
  `TASK_NOT_EXECUTABLE`，附 `cargo tree`/`cargo metadata` 证据。
- 兄弟 ownership 冲突：先通知 root。

## 执行结果（提交前自验收）

- `cargo test -p skiff-runtime-request-contract -p skiff-runtime-transport -p skiff-router`：
  request-contract 3 passed；transport 113+2 passed；router 7 passed（含 contracts 3）。
- `cargo test -p skiff-runtime-model -p skiff-runtime-capability-context`：100+71 passed。
- `cargo check --workspace`：通过。
- `node scripts/verify.mjs --only router-rust`：`router-rust:contracts` passed。
- `cargo tree -p skiff-router -e normal` 负例断言：不含 `skiff-runtime-model` /
  `skiff-runtime-host` / `skiff-runtime-eval` / request execution。
- rustfmt：触碰文件已 `cargo fmt`；clippy：触碰 crates 无新增 error（既有 advisory warning 保留）。
