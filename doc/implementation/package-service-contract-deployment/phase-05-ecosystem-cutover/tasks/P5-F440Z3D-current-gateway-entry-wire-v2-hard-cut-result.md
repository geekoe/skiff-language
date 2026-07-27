# P5-F440Z3D Current GatewayEntry request wire v2 hard cut result

状态：`IMPLEMENTED / IN_SCOPE_PASS / BASELINE_NON_LIVE_BLOCKED`。

本 leaf 已把 current RuntimeAssembly HTTP、test-control、WebSocket connect 与 WebSocket
JSON-RPC request 的 `routing.gatewayEntryIdentity`，以及两个 WebSocket metadata 中的同值
identity，统一 hard cut 为：

```text
skiff-gateway-entry-v2:sha256:<64 lowercase hex>
```

没有 dual read、prefix 转换、Rust production 或 Gateway/dispatcher/snapshot/broker/Endpoint
改动。Router 的规定 301 个测试、type-check、可进入目标的七条 Rust 命令与静态检查均通过。

完整规定验证仍有两个基线阻塞，均需要越过本 leaf 明确写集才能修复：

1. `skiff-runtime-package-test` 的 test support 缺少两个已有
   `collection_name_mapping` initializer，目标测试未能编译；
2. 任务规定的无参 cross-system verifier 与当前 CLI 不一致；额外运行其
   `--runtime-wire-self-test` 后，current request corpus 全部通过，但随后在本任务禁止改动的
   legacy request `ServiceProtocolIdentity v3` fixture 上失败。

因此本 result 不把完整 non-live gate 宣称为 PASS，也不返回 `TASK_SCOPE_EXPANDED`：hard cut
本身不需要第四个 production owner，Rust production 仍是统一 v2。

## 1. 基线、分支与提交

| 项目 | 值 |
| --- | --- |
| worktree | `/Users/geek/workspace/skiff-p5-f440z3d-gateway-wire-v2` |
| branch | `codex/p5-f440z3d-gateway-wire-v2` |
| implementation baseline | `50260b5ee6e031c735824eef411fabcbc3847461` |
| task start HEAD | `aa9d3580` |
| implementation commit | `73db114a92e8594ad848b54600a131405aa534e1` |
| result commit | 本文件独立提交，见 branch history |

implementation commit 精确包含任务允许的 21 个 production/test/corpus 文件；result 未混入
implementation commit。

## 2. 真实 test-first 证据

production 修改前先把两个 current-positive probe 切到 v2，并分别执行。

### 2.1 HTTP production builder RED

`router/tests/assembly-replica-dispatch.test.ts` 的真实
`RuntimeAssemblyIngressBinding` 使用 current v2 后调用
`assemblyHttpRequestHeader`：

```bash
router/node_modules/.bin/vitest run --root router \
  tests/assembly-replica-dispatch.test.ts
```

结果：exit `1`，`1 failed`。失败来自 production builder 内的 production validator：

```text
Error: invalid request.start envelope:
routing.gatewayEntryIdentity must be
skiff-gateway-entry-v1:sha256:<64 lowercase hex>
at assemblyHttpRequestHeader (src/router/assemblyHttpGateway.ts:269)
```

### 2.2 WebSocket connect production validator RED

`router/tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts` 构造 routing 与
`websocketConnect.gatewayEntryIdentity` 都为 current v2 的 connect header，并直接调用
`validateRuntimeAssemblyRequestStartFrameWireHeader`：

```bash
router/node_modules/.bin/vitest run --root router \
  tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts \
  -t "keeps method null on websocketConnect"
```

结果：exit `1`，production validator 返回 `{ ok: false }`，与测试要求的 `{ ok: true }`
不符。

三处 production owner 修改后，两条同名 probe 均通过；不是 synthetic throw、字符串静态
断言或只改 corpus 制造的 RED。

## 3. Implementation

### 3.1 三个且仅三个 production owner

- `router/src/protocol/runtimeAssemblyRequest.ts`
  - 删除 v1 legacy GatewayEntry pattern；
  - 删除 `websocketJsonRpc ? v2 : v1` 分流；
  - HTTP、test、connect、JSON-RPC routing 统一只接受 v2；
  - diagnostic 统一声明 current v2。
- `router/src/protocol/runtimeAssemblyRequestMetadata.ts`
  - connect metadata 改为 v2；
  - connect/JSON-RPC routing 与 metadata exact equality 保持不变。
- `router/src/protocol/runtimeProtocol.ts`
  - HTTP routing schema 从任意 string 收紧到 v2；
  - connect routing 与 metadata schema 从 v1 改为 v2；
  - JSON-RPC v2 schema 保持不变。

### 3.2 Test 与 corpus

- 列出的 Router 与 Rust current-positive/incidental GatewayEntry fixture 全部改为 v2。
- 所有 mismatch fixture 使用合法但不同 digest 的 v2；direct assertions 确认 connect 与
  JSON-RPC mismatch 命中 exact-equality diagnostic，而不是 generation parser。
- `protocol.test.ts` 直接证明 HTTP、connect routing、connect metadata、JSON-RPC routing 与
  JSON-RPC metadata schema 拒绝 stale v1；current v2 positive 均通过。
- HTTP corpus 新增 `stale GatewayEntry v1 generation`。
- connect corpus 分别新增：
  - `stale GatewayEntry v1 routing generation`；
  - `stale GatewayEntry v1 metadata generation`。
- connect corpus 的 `mismatched gateway identity` 使用两个合法且不同的 v2 digest。
- HTTP corpus 的 uppercase、short 与 raw 非 generation negatives 都改用 v2 base。
- `runtime/request/src/http_gateway_target.rs` 中原有 WebSocket-surface negative 的 helper
  缺少 current 必需的 `jsonrpc-2.0-text` profile，导致规定的 HTTP Rust filter 在目标断言前
  失败；只在该任务允许的 test-only 文件补齐 current profile，保持原
  `PlanSurfaceMismatch` 断言与 production 不变。重跑后通过。

以下既有 deliberate stale-generation negative owner 保持未改：

- `artifact-identity/src/tests/gateway.rs`
- `artifact-identity/src/tests/deployment.rs`
- `router/tests/filesystem-runtime-assembly-snapshot-loader.test.ts`
- `router/tests/runtime-assembly-websocket-rpc-snapshot.test.ts`

反向搜索确认三个 production owner 不再含
`skiff-gateway-entry-v1` 或 legacy GatewayEntry pattern。写集内剩余 v1 只出现在命名明确的
HTTP/connect/JSON-RPC stale-generation negatives 与 schema rejection probes。

## 4. Non-live 验证

### 4.1 Router

规定的 `vitest list` 原样执行成功；独立 `wc -l` 记录 non-zero count 为 `301`。

规定的 14 文件命令最终结果：

```text
Test Files  14 passed (14)
Tests       301 passed (301)
```

```bash
pnpm --dir router type-check
```

结果：PASS。

### 4.2 Rust shared target

所有命令都使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 规定命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-artifact-identity gateway_identity_marker_parser_and_preimage_match_exact_golden` | PASS，1 passed |
| `cargo test -p skiff-artifact-identity deployment_identity_is_stable_under_reorder_and_rejects_stale_generation` | PASS，1 passed |
| `cargo test -p skiff-runtime-transport runtime_assembly_request` | PASS，19 passed |
| `cargo test -p skiff-runtime-request runtime_http_gateway` | 最终 PASS，9 passed |
| `cargo test -p skiff-runtime-request websocket_connect_request` | PASS，2 passed |
| `cargo test -p skiff-runtime-activation activation_context_websocket_entry_is_typed_optional_and_matches_all_exact_facts` | PASS，1 passed |
| `cargo test -p skiff-runtime-package-test --test package_artifact entrypoint_validation_rejects_non_exact_gateway_facts` | `BASELINE_BLOCKED`，未进入目标测试 |
| `cargo test -p skiff-runtime-host websocket_admission_rejects_gateway_identity_and_surface_mismatch` | PASS，1 passed |

package-test 的精确编译错误是：

```text
error[E0063]: missing field `collection_name_mapping` in initializer of
`PackageRequirement`
  --> runtime/package-test/tests/support/mod.rs:732:36

error[E0063]: missing field `collection_name_mapping` in initializer of
`PackageBinding`
  --> runtime/package-test/tests/support/mod.rs:809:5
```

`50260b5e` 中同一 support 文件的两个 initializer 已经缺少该字段；本 branch 对该文件无 diff。
它不在本任务 test 写集内，因此没有越界修复。

### 4.3 Cross-system 与静态检查

任务规定的命令已原样执行：

```bash
node cross-system-fixtures/package-service-ecosystem/verify.mjs
```

当前 verifier 要求恰好一个 selector，因此返回：

```text
usage: node verify.mjs
<--self-test|--combined-probe|--runtime-wire-self-test>
```

没有修改任务明确禁止修改的 checker。额外执行最相关的 non-live selector：

```bash
node cross-system-fixtures/package-service-ecosystem/verify.mjs \
  --runtime-wire-self-test
```

该函数先完成 current HTTP request positive、mutation、raw、payload 与 equivalent-option
corpus 的 production validator/decoder 检查；随后在 `verify.mjs:441` 的 legacy-only loop
失败：

```text
invalid request.start envelope:
serviceProtocolIdentity must be
skiff-service-protocol-v5:sha256:<64 lowercase hex>
```

对应 legacy fixture 仍携带既有 v3。修改它既不是 GatewayEntry identity 刷新，也被任务的
“不得顺手改其它 fixture/checker”禁止，所以只记录基线阻塞。Router 的 shared connect corpus
与 Rust transport shared corpus tests 均已通过。

另外：

```text
rustfmt --edition 2021 --check <6 changed Rust test files>  PASS
git diff --check                                             PASS
```

## 5. 完成矩阵

| 条款 | 证据 | 结论 |
| --- | --- | --- |
| 四条 current branch 的 TS lexical/schema 与 Rust current parser 一致 | 三个 production owner 均为 v2；Router 301 tests；Rust transport 19 tests | PASS |
| current v2 HTTP builder 与 connect validator 通过 | 两个真实 RED 在 production 修改后转绿 | PASS |
| v1 在 HTTP/connect/JSON-RPC routing/metadata fail closed | shared corpus mutations、protocol schema probes、JSON-RPC direct mutations | PASS |
| mismatch 到达 exact equality | connect/JSON-RPC exact diagnostic assertions；合法不同 v2 digest | PASS |
| shared corpus positive 全部 v2且 stale generation 独立 | 两份 corpus + Router/Rust corpus consumers | PASS |
| Rust production 与 Z3B Gateway 不变 | implementation commit 仅三个 Router production owner；其余均 test/corpus | PASS |
| 规定 non-live 命令全部绿色 | package-test support 与 verifier/legacy fixture 两个基线阻塞 | BLOCKED |

## 6. Scope 与操作约束

- 未修改第四个 production owner；
- 未修改 Rust production；
- 未修改 Gateway、dispatcher、snapshot、broker、Endpoint、artifact producer、public 配置或
  其它 wire 字段；
- 未启动 stable instance、watch、MongoDB、server、live 或完整 suite；
- 未访问外部 network；
- 未派子 Agent；
- 未 merge、rebase 或 push。

implementation invariant 已完成。若要把规定 non-live gate 也变为全绿，需要由写集 owner
单独修复 package-test support，并统一 verifier 调用契约与 legacy ServiceProtocol fixture；
这些不应混入本 hard-cut implementation commit。
