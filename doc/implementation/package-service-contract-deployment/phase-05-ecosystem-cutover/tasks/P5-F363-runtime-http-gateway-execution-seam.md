# P5-F363 Runtime HTTP gateway execution seam

状态：Ready（C3 shared Rust request/eval leaf；Host admission/wire 在本检查点之后接入）。

## 直接父节点

- `P5-H36-external-ingress-implementation-dag.md`
- `P5-F358-runtime-assembly-http-gateway-linking-result.md`
- `P5-F359-http-gateway-request-protocol-result.md`
- `P5-F360-typed-json-unary-correction-result.md`
- `P5-F361-package-test-gateway-entrypoint-result.md`

父节点已经冻结 HTTP-only canonical wire、linked gateway exact facts、typedJson unary 与 rawHttp
server-stream 边界。本任务只建立 request/eval 的 exact gateway execution seam；不修改 Host wire、
Router、transport protocol、test-runner producer 或 artifact。

## Exact base

- integration commit：`b71e622ca35109519e904f269a67f19bc2f08de4`
- integration tree：`7d79c140534db0ed2336e3babe511fa444fdc2e6`
- branch：`codex/package-service-phase-05`

当前 `RuntimeAssemblyRequestTarget` 只携带 `RuntimeAssemblyServiceCallTarget`，
`execute_runtime_assembly_request` 只会按 `BoundaryOperationDescriptor` 执行 service operation。
虽然 F358 已经提供 exact linked gateway handler/pre/guard、signature、adapter plan 与 protocol surface，
request/eval 尚无不经过 ServiceContract/ContractOperationId 的执行入口。

## 必须完成

1. 建立职责准确的 Runtime HTTP gateway target：
   - pin 同一个 `RuntimeAssemblyEvalTarget`、request activation 与 exact gateway owner；
   - 保存并逐值验证 `GatewayEntryKey`、`GatewayEntryIdentity`、HTTP protocol surface、adapter plan；
   - handler/pre/guard 只接受 F358 已 linked 的 exact callable target 与 signature；
   - execution image 必须包含同一 exact target，不得按 display/source/public symbol、短名或
     ServiceContract operation 回退。
2. HTTP gateway target 与现有 internal service-call target 是两个并列入口。不得把 gateway
   伪装成 `RuntimeAssemblyServiceCallTarget`，不得合成 `ContractOperationId`、
   `BoundaryOperationDescriptor`、service caller 或 service protocol identity。
3. request/eval 通过 deployment-owned adapter plan 执行：
   - `typedJson` unary：从 opaque request body 按 exact linked handler signature 解码，执行可选
     guard/pre 与 handler，再编码 JSON response；
   - `rawHttp` unary：构造 `std.http.HttpRequest`，执行同一 plan，返回 status/headers/body；
   - `rawHttp` server stream：执行
     `Stream<std.http.HttpResponseStreamEvent>`，只发 start/chunk/end；
   - `typedJson + serverStream`、raw stream错误事件序列、mode/surface/plan不一致全部 fail closed。
4. 复用已有 HTTP boundary、heap type plan、stream cleanup、cancellation、response ceiling 与
   single-terminal owner。不得新增另一套 JSON codec、HTTP type layout、stream scheduler 或 payload
   framing。
5. canonical HTTP gateway request必须具有 binary HTTP context；body bytes保持opaque直到 Eval adapter。
   canonical wire中没有 `httpAdapter`、`testEffectDoubles`或WebSocket metadata。内部 package-test /
   legacy request fixture若仍需要自己的 test-effect sequence，不得把它重新放回 canonical wire。
6. 为后续 Host 暴露最小 typed API：Host只需把 admitted `LinkedGatewayEntry`、activation/eval pin、
   request metadata/body与mode交给本 seam；不得要求 Host 重新解释 adapter args、schema或 callable
   signature。
7. 更新 request/eval 直接 tests，至少证明：
   - typed unary、raw unary、raw server stream真实调用 exact private handler；
   - guard short-circuit、pre context与handler target保持exact；
   - wrong owner/key/identity/target/signature/mode/adapter kind fail closed；
   - cancellation、stream cleanup、single terminal不回归；
   - internal service operation执行继续存在且与gateway target互不替代。

## 写入范围

允许：

- `runtime/request/**`；
- `runtime/eval/**`；
- 上述 crate 为最小 typed seam 所需的 `Cargo.toml` 依赖；
- 直接聚焦 fixtures/tests。

禁止：

- `runtime/host/**`、`runtime/transport/**`、`runtime/loader/**`、`runtime/linker/**`；
- artifact/deployment/identity/compiler；
- Router、test-runner、三仓库 service、stable/live 配置、lockfile。

若正确执行必须修改 F358 linked entry 公共事实或 F359 wire，立即返回
`TASK_SCOPE_EXPANDED`。

## 验证

先枚举 selector 并确认非零，再运行与实际新增测试匹配的聚焦命令：

```bash
cargo test -p skiff-runtime-eval <gateway-selector> -- --list
cargo test -p skiff-runtime-eval <gateway-selector>
cargo test -p skiff-runtime-request <gateway-selector> -- --list
cargo test -p skiff-runtime-request <gateway-selector>
cargo check -p skiff-runtime-eval -p skiff-runtime-request
rustfmt --edition 2021 --check <changed-rust-files>
git diff --check
```

production seam反向搜索不得包含
`ContractOperationId|ServiceContractRef|RuntimeAssemblyServiceCallTarget|contract_operation_id`。
不运行 workspace/root、Host、stable/live，不push。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f363-runtime-http-gateway-seam`
- branch：`codex/p5-f363-runtime-http-gateway-seam`
- 从包含本task的integration checkpoint创建；
- production/tests一个commit，result一个commit；
- result写入`P5-F363-runtime-http-gateway-execution-seam-result.md`；
- worktree保持clean，不merge/rebase integration，不push。
