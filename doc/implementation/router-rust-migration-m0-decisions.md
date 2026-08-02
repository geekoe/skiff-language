# Router Rust Migration M0 Decision Record

日期：2026-08-02
相关批次：`doc/implementation/router-rust-migration-batch-2.md`（M0）
权威设计：`doc/implementation/router-rust-migration-plan.md` §2.3 / §5.3 / §5.5 / §6.1

## M0-D1：wire/service-error facts 下沉到 `skiff-runtime-request-contract`

现状：`skiff-runtime-transport -> skiff-runtime-model`，且 `skiff-runtime-request-contract ->
skiff-runtime-capability-context -> runtime-model/boundary/native-contract/tokio`。

决策：

- `runtime/request-contract` 成为低层 wire/request-contract owner，移除对
  `skiff-runtime-capability-context` 的依赖。
- 下沉内容：`addr`（TypeAddr 等）、`error`（RuntimeErrorPayload/WirePayload）、
  `service_error`（OpaqueServiceError/ServiceErrorEnvelope/identity）、`ActorRef`、
  `ActorInvocationDeclarationOwner`、outbound control DTO、`OutboundResponse`、
  `response`（HttpNameValue/ResponseError/FixedServiceResponseFailure 等）。
- `runtime-model` 与 `runtime-capability-context` 对被下沉类型 re-export，保持既有
  `skiff_runtime_model::*` / `skiff_runtime_capability_context::*` public surface。
- `skiff-runtime-transport` 移除 `skiff-runtime-model` 依赖。

证据：`cargo tree -p skiff-router -e normal` 只含
`skiff-runtime-request-contract`/`skiff-runtime-transport` 及基础依赖，
无 `skiff-runtime-model`/runtime-host/eval/request execution。

## M0-D2：transport protocol 机械拆分

- `protocol.rs` 收敛为 closed family registry + re-export + 稳定 sink registration contract。
- `protocol/frame.rs`：binary frame codec。
- `protocol/session.rs`：session/bootstrap/health。
- `protocol/request.rs`：request/response DTO。
- `protocol/spawn.rs`：spawn family。
- `protocol/actor.rs`：actor control frames + test authority validation。
- `protocol/control.rs`：control/telemetry（不归 Router lane，仅机械移出）。

wire bytes 与 golden corpus 不变（transport 113 个 unit test + 2 个 integration test 全过）。

## M0-D3：closed frame-family registry 与 sink contract

- `RuntimeFrameFamily` closed enum：Session/Request/Activation/Connection/Actor/Spawn。
- `RUNTIME_FRAME_FAMILY_RULES`：每个 family 的 direction 与 payload presence。
- `RuntimeFrameSink` + `RuntimeFrameSinkRegistration` + `RuntimeFrameSinks`：
  按 §5.5 冻结稳定 sink bundle，demux 主体留给后续 lane。
- 新增 family 必须同时改 enum、rules、sink bundle 与 family module。

## M0-D4：Router consumer gate

- `router/Cargo.toml` 仅增加 `skiff-runtime-transport` 与 `skiff-runtime-request-contract`。
- `router/tests/contracts.rs` 作为 `router-rust:contracts` 的窄 consumer：
  编译共享 envelope/connection identity、断言 closed registry 与 sink registration。
- `node scripts/verify.mjs --only router-rust` 通过。
