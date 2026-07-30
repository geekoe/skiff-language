# P5-F440Z3D Current GatewayEntry request wire v2 hard cut

状态：Ready。共享wire checkpoint；一次hard cut所有RuntimeAssembly gateway request branch到
GatewayEntry v2，并解除Z3B。

## 直接父节点

- `P5-F440Z3C-current-gateway-entry-wire-identity-preflight-result.md`

父审计已经沿真实producer/consumer证明：

- current HTTP、test-control、WebSocket connect producer都携带snapshot的v2 identity；
- TypeScript只有WebSocket JSON-RPC branch已接受v2，其余三条仍要求v1；
- Rust共同typed parser已经只接受v2；
- 只改connect会留下HTTP/test真实入口必断，不能形成绿色checkpoint。

实现基线为 `50260b5e` 对应的current integration tree。

## 目标与唯一不变量

所有current RuntimeAssembly request的
`routing.gatewayEntryIdentity`，以及两个WebSocket metadata中的同值identity，统一只接受：

```text
skiff-gateway-entry-v2:sha256:<64 lowercase hex>
```

覆盖：

- HTTP unary / server stream；
- test-control构造的HTTP request；
- WebSocket connect；
- WebSocket JSON-RPC（保持既有v2）。

不得dual read、不得prefix转换、不得保留current-positive v1 fixture。错误维度不是generation的
negative fixture必须改成“合法但不同的v2”，避免被lexical错误提前遮挡。

## DAG位置

```text
Z3C preflight
  -> Z3D all-branch v2 wire checkpoint
  -> Z3B Gateway/server production hookup（重新调度）
  -> F0剩余fixtures/tooling
```

完成后仍是实现检查点，不是稳定候选。

## 唯一production写集

只允许修改：

1. `router/src/protocol/runtimeAssemblyRequest.ts`
   - 删除legacy GatewayEntry pattern与 `websocketJsonRpc ? v2 : v1` 分流；
   - 所有routing branch统一验证current v2；
   - diagnostic明确current v2。
2. `router/src/protocol/runtimeAssemblyRequestMetadata.ts`
   - `websocketConnect.gatewayEntryIdentity`改为current v2；
   - 保留routing/metadata exact equality；
   - JSON-RPC v2不变。
3. `router/src/protocol/runtimeProtocol.ts`
   - HTTP routing schema由任意string收紧到v2 pattern；
   - connect routing与metadata两处v1改v2；
   - JSON-RPC v2不变。

禁止修改Router Gateway/dispatcher/snapshot/broker/Endpoint、Rust production、artifact producer、
public配置或其它wire字段。

## Test与fixture写集

Router current-positive/incidental fixture：

- `router/tests/assembly-http-gateway-stream.test.ts`
- `router/tests/assembly-replica-dispatch.test.ts`
- `router/tests/assembly-runtime-endpoint.test.ts`
- `router/tests/router-websocket-trust-dispatch.test.ts`
- `router/tests/runtime-assembly-unary-dispatch.test.ts`
- `router/tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts`
- `router/tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts`
- `router/tests/service-error-cross-layer-convergence.test.ts`
- `router/tests/websocket-gateway.test.ts`
- `router/tests/protocol.test.ts`

Rust current-positive/incidental fixture（test-only）：

- `runtime/activation/src/tests.rs`
- `runtime/host/src/loader/active_assembly_context.rs`
- `runtime/package-test/tests/package_artifact.rs`
- `runtime/request/src/http_gateway_execution/tests.rs`
- `runtime/request/src/http_gateway_target.rs`
- `runtime/request/src/websocket_connect_execution.rs`

共享corpus：

- `cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json`
- `cross-system-fixtures/package-service-ecosystem/runtime-websocket-connect-wire.json`

只允许在这些文件内机械刷新identity、canonical JSON和直接断言；不得顺手改其它fixture、
README/checker或result。

以下明确的stale-generation negative必须保留v1：

- `artifact-identity/src/tests/gateway.rs`
- `artifact-identity/src/tests/deployment.rs`
- `router/tests/filesystem-runtime-assembly-snapshot-loader.test.ts`
- `router/tests/runtime-assembly-websocket-rpc-snapshot.test.ts`

共享corpus中原uppercase/short/mismatch等非generation negative应先换成v2 base，再新增命名明确的
stale-v1 generation mutation：

- HTTP corpus至少一个routing stale-v1；
- connect corpus分别有routing stale-v1与metadata stale-v1；
- connect mismatch继续使用两个合法但不同的v2 digest。

## Test-first

production修改前先建立至少两个真实RED：

1. current v2 HTTP binding调用production header builder，当前在TS lexical gate失败；
2. current v2 WebSocket connect header调用production validator，当前在同一gate失败。

必须真实执行、非零匹配并记录失败。不得用synthetic throw、纯字符串pattern断言或只改fixture制造RED。

## 完成标准

- HTTP、test-control、connect、JSON-RPC四条branch的TS lexical/schema与Rust current parser一致；
- current v2 HTTP和connect header可从真实production builder通过；
- v1在HTTP、connect routing、connect metadata、JSON-RPC routing/metadata均fail closed；
- routing与metadata mismatch仍命中exact equality，而非generation parser；
- shared corpus positive全部v2，stale v1 mutation明确且各层不互相遮挡；
-所有声称测试identity mismatch/uppercase/其它字段的fixture使用能到达目标断言的current v2 base；
-不改变Rust production或Z3B Gateway代码。

## 必跑non-live验证

先列出Router tests并记录non-zero count：

```bash
router/node_modules/.bin/vitest list --root router \
  tests/protocol.test.ts \
  tests/runtime-assembly-request-wire.test.ts \
  tests/runtime-protocol-websocket-response.test.ts \
  tests/assembly-http-gateway-stream.test.ts \
  tests/assembly-replica-dispatch.test.ts \
  tests/assembly-runtime-endpoint.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  tests/router-websocket-trust-dispatch.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts \
  tests/websocket-gateway.test.ts \
  tests/service-error-cross-layer-convergence.test.ts \
  tests/filesystem-runtime-assembly-snapshot-loader.test.ts \
  tests/runtime-assembly-websocket-rpc-snapshot.test.ts
router/node_modules/.bin/vitest run --root router \
  tests/protocol.test.ts \
  tests/runtime-assembly-request-wire.test.ts \
  tests/runtime-protocol-websocket-response.test.ts \
  tests/assembly-http-gateway-stream.test.ts \
  tests/assembly-replica-dispatch.test.ts \
  tests/assembly-runtime-endpoint.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  tests/router-websocket-trust-dispatch.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts \
  tests/websocket-gateway.test.ts \
  tests/service-error-cross-layer-convergence.test.ts \
  tests/filesystem-runtime-assembly-snapshot-loader.test.ts \
  tests/runtime-assembly-websocket-rpc-snapshot.test.ts
pnpm --dir router type-check
```

Rust使用共享target：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-artifact-identity gateway_identity_marker_parser_and_preimage_match_exact_golden
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-artifact-identity deployment_identity_is_stable_under_reorder_and_rejects_stale_generation
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-transport runtime_assembly_request
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-request runtime_http_gateway
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-request websocket_connect_request
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-activation activation_context_websocket_entry_is_typed_optional_and_matches_all_exact_facts
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-package-test --test package_artifact entrypoint_validation_rejects_non_exact_gateway_facts
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-host websocket_admission_rejects_gateway_identity_and_surface_mismatch
```

最后：

```bash
node cross-system-fixtures/package-service-ecosystem/verify.mjs
git diff --check
```

不得启动stable instance、watch、外部network、live或完整suite。

## 停止与交付

若hard cut需要修改三个production owner以外的生产代码，或Rust production并非统一v2，停止并返回
`TASK_SCOPE_EXPANDED`。若只是本任务列出的current-positive fixture跟随，不算扩张。

- worktree：`/Users/geek/workspace/skiff-p5-f440z3d-gateway-wire-v2`
- branch：`codex/p5-f440z3d-gateway-wire-v2`
- result：`P5-F440Z3D-current-gateway-entry-wire-v2-hard-cut-result.md`

Implementation与result分开提交。5分钟内开始真实test-first修改；不得派子Agent，不得
merge/rebase/push。
