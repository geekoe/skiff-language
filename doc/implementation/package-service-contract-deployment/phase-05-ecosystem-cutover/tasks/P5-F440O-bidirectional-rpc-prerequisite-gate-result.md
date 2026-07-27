# P5-F440O Bidirectional WebSocket RPC prerequisite gate result

状态：`PASS / P0_PREREQUISITES_PRESENT / FOLLOWER_BLOCKERS_ROUTED`。

External manifest / JSON-RPC authoring、artifact identity、deployment projection checkpoint 与
cancellation hard cut 同时存在于任务指定输入。没有 checkpoint production 回归，也没有 production
公开 `CancelError` consumer。下文的 T0/E0/R0 缺口都是 F440B 已分配的后继工作，不阻止 T0 启动。

## 1. 精确输入、当前 tree 与父提交

验收输入与本 gate 的执行代码状态分别为：

| 状态 | Commit | Tree |
| --- | --- | --- |
| 任务指定验收输入 | `d31b4e7f28ef415c61c9e4ada2a1168703d4adcf` | `e55dd42041e0b1f84f233873d221586b6815a286` |
| gate task HEAD / 实际执行状态 | `d51b367d8b8d0029d50f1d3811745ffbf9e932d9` | `4d6b69baf7c3d2819f6211e43b54896b3010a45e` |

`d31b4e7f` 是 `d51b367d` 的直接父提交；两者 diff 只新增本 gate task 文档，不含 production、test 或
fixture 变化。因此所有命令都验证同一 production/test tree。

五个直接父节点在 integration history 中的 implementation/result commit 如下，全部是验收输入
`d31b4e7f` 的 ancestor（输入自身按 ancestor 计）：

| 直接父节点 | Integration implementation commit / tree | Result commit / tree | Input ancestor |
| --- | --- | --- | --- |
| F440B owner audit | 无 implementation；只读审计 | `5a24c90a538c835c9580466515cadd37b4a41dd3` / `128eda5ddf8b613469d5d43b4b525cc3c6acfa98` | yes |
| F440M manifest identity follower | `a3fb8cc641fd6e3743d47e5e22ee24118e0d399e` / `8b769169cca2fc306ecc90931b490a3a112dcbb1` | `67d61b8db9cb1750fe624dc40b9968642fb6d7f3` / `6ffd7924e0e7359e3ffd2f05635bd724a2d961ff` | yes / yes |
| F440K request/Host/transport cancellation | `83e9dfbb729fc5f2597f3236810af04d1ad4055f` / `4bf479ad52c93987315a94e6a34d885dde15b1a0` | `aa14721be58646492a84ea7541a0a1d3a197ca01` / `7f145203fa5f620cddc1911818278e109ac619ac` | yes / yes |
| F440N runtime model cancellation cleanup | `19f59434a4cc1caad9899c57c8e988cf8c86dd69` / `088b6e5b5b236478c40b81cb05100362e76f2de4` | `d31b4e7f28ef415c61c9e4ada2a1168703d4adcf` / `e55dd42041e0b1f84f233873d221586b6815a286` | yes / yes |
| F440J Router cancellation projection | `31e9fad607c7dc862c207d8ade303b088e4d63cc` / `a3e111184cee3e5751540a81993f7a53dcf5f740` | `d12b15d42cd3bb7b9e0818f2a0c0c82a9b0760e4` / `c6e268cabc835e647972c198a140adc60fa28a35` | yes / yes |

父 result 记录的是 task worktree 原 implementation hash；integration branch 使用 patch-equivalent
cherry-pick，所以原 hash 本身不是 integration ancestor。逐项 `git patch-id --stable` 证明：

| Task implementation | Integration implementation | Stable patch id | Equal |
| --- | --- | --- | --- |
| `b0ae32afbe5e50bd22b595ee0dba1c37f106ad5e` | `a3fb8cc641fd6e3743d47e5e22ee24118e0d399e` | `81a27a1bdbf2fc2d2a039925fe2580bb7b66d36e` | yes |
| `d1d1d174163843ac78af4c68d6c5b6611efbee9b` | `83e9dfbb729fc5f2597f3236810af04d1ad4055f` | `cccc01446ef69fb76081f20629d5e95df9361780` | yes |
| `d435ea95994173b0dcfc11d5478b9d1c57b37454` | `19f59434a4cc1caad9899c57c8e988cf8c86dd69` | `7d39b9736b35de4e29e3a14c9d5b4691ec5fbf8d` | yes |
| `72c1294207d3fdf763459b3c8759dbe325309690` | `31e9fad607c7dc862c207d8ade303b088e4d63cc` | `72f89dea158aea1be9ac2743b59124efd7bc5816` | yes |

## 2. Listing 与执行结果

所有 Cargo 命令统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

### 2.1 必跑 Rust matrix

| 命令 | 实际执行计数 | 结果 |
| --- | ---: | --- |
| `cargo test -p skiff-artifact-model gateway` | 10 | PASS：10 passed、165 filtered |
| `cargo test -p skiff-artifact-identity gateway` | 17 | PASS：17 passed、117 lib filtered；其它 test binary 0 executed |
| `cargo test -p skiff-artifact-identity deployment` | 8 | PASS：8 passed、126 lib filtered；其它 test binary 0 executed |
| `cargo test -p skiff-deployment` | 61 | PASS：61 passed |
| `cargo test -p skiff-compiler --test websocket_ingress` | 10 | PASS：10 passed |
| `cargo test -p skiff-runtime-model` | 88 | PASS：88 passed |
| `cargo test -p skiff-runtime-capability-context` | 40 | PASS：38 unit + 2 compile-fail doctest |
| `cargo test -p skiff-runtime-transport` | 83 | FOLLOWER BLOCKED：79 passed、4 failed；Cargo 在 lib failure 后未执行剩余 integration binaries |

必跑 Rust 命令实际执行 `317` 条：`313 passed / 4 failed`。四个 failure 全部位于
`runtime/transport/src/runtime_assembly_request/tests.rs`：

- `runtime_assembly_request_current_http_and_websocket_json_match_shared_goldens`
- `runtime_assembly_request_start_normalizes_equivalent_optional_defaults`
- `runtime_assembly_request_start_decodes_shared_http_headers`
- `runtime_assembly_request_start_preserves_opaque_http_payload_boundaries`

它们分别在当前 v2 reader 读取
`cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json` /
`runtime-websocket-connect-wire.json` 的旧
`skiff-gateway-entry-v1:sha256:*` 时 fail closed。首错都是
“must use `skiff-gateway-entry-v2:sha256:<64 lowercase hex>`”，没有 production compile error、
cancellation payload 或 ordinary-response regression。

补充精确 cancellation probe：

```text
cargo test -p skiff-runtime-transport \
  cancellation_terminal_cannot_be_encoded_as_response_error_but_ordinary_failures_can
```

结果：`1 passed / 82 filtered`；两个 integration binaries共 `2 filtered`。这直接证明 transport
cancellation terminal 仍不能编码为普通 `response.error`。

### 2.2 Router listing 与 execution

本 worktree 没有 `router/node_modules`，PATH 中也没有 `pnpm`；没有安装依赖。直接借用 integration
worktree 已存在的依赖时，未链接的首次 listing 因当前 worktree 无法解析 `ws`/`yaml` 而退出。随后仅在
单个 shell 生命周期内建立
`router/node_modules -> /Users/geek/workspace/skiff-phase-05-integration/router/node_modules`
只读链接，并以 `trap` 删除。

`router/package.json` 的底层 runner 是 Vitest。为避免 bare `cancellation` 被解释成 file filter并得到
零测试，实际先 listing：

```text
router/node_modules/.bin/vitest list --root router --exclude 'dist/**' \
  tests/protocol.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  -t cancellation
```

listing 精确得到 `4` 个非零 selector：

- protocol legacy ordinary cancellation rejection：1
- unary legacy fixed/control cancellation rejection与 no-499：2
- bounded Router shutdown control cancellation：1

随后使用相同文件/name selector执行：

```text
router/node_modules/.bin/vitest run --root router --exclude 'dist/**' \
  tests/protocol.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  -t cancellation
```

结果：`2 files passed`，`4 passed / 66 skipped`，两文件 inventory 共 `70`。临时链接已删除。

### 2.3 静态与只读 compile probes

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |
| `cargo check -p skiff-runtime-eval` | EXPECTED FOLLOWER BLOCK：3 个 `E0004`，见第4节 |
| `cargo check -p skiff-runtime-host` | EXPECTED FOLLOWER BLOCK：先在 eval dependency 命中同3个 `E0004` |

## 3. 反向搜索分类

### 3.1 Cancellation public surface

规定搜索共 `17` 行：

```text
rg -n 'CancelError|PlatformBuiltinErrorIdentity::Cancel' \
  artifact-model compiler runtime router --glob '!**/README.md'
```

分类如下：

| 分类 | 行数 | 路径 / 结论 |
| --- | ---: | --- |
| Production admission tombstone | 1 | `artifact-model/src/file_ir.rs:101`；只拒绝 retired spelling，不导出、不 lower |
| Artifact negative tests | 2 | `artifact-model/src/file_ir/legacy_builtin_tests.rs` |
| Compiler negative tests | 2 | `compiler/tests/builtin_canonical_spelling.rs` |
| Eval linked-spelling negative test | 2 | `runtime/eval/src/assembly_execution/projection.rs:699,704` |
| Runtime-model finite-registry negative tests | 4 | `runtime/model/src/service_error.rs:594,602,610,617` |
| Router negative rejection / no-499 tests | 5 | `router/tests/protocol.test.ts` 与 `runtime-assembly-unary-dispatch.test.ts` |
| Stale test-only enum consumer | 1 | `runtime/eval/src/assembly_execution/service_error_channel/tests.rs:127`；已删除 enum member，因此不能构造 production identity |

Production public error identity、production ordinary response materialization和 production
`PlatformBuiltinErrorIdentity::Cancel` 均为 `ZERO_MATCHES`。唯一 production spelling 是显式 retired
admission tombstone；control `request.cancel` 仍由通过的 transport/Router tests 保留。

### 3.2 JSON-RPC shared checkpoint

规定搜索共 `108` 行，`68 production / 40 tests`：

```text
rg -n 'WebSocketJsonRpc|websocketJsonRpc' \
  artifact-model artifact-identity deployment compiler
```

| Owner | Production | Tests | 覆盖 |
| --- | ---: | ---: | --- |
| Authoring | 8 | 0 | `artifact-model/src/ecosystem_authoring.rs` 的 strict method map / DTO |
| Artifact surface / exports | 24 | 0 | `artifact-model/src/gateway.rs` + `lib.rs` 的 kind、source、protocol surface |
| Identity / deployment validation | 15 | 31 | `artifact-identity/src/gateway.rs`、`deployment/validation.rs` 及 gateway/deployment tests |
| Compiler admission / projection | 21 | 9 | compiler input、WebSocket projection、HTTP rejection及两组 integration tests |
| 合计 | 68 | 40 | 108 |

`deployment/**` 不复制 enum spelling；它消费已验证的 typed gateway entries。其完整 `61 passed`
projection/assembly matrix与 artifact-identity deployment `8 passed` 一起证明 deployment owner已经消费
当前 surface。搜索不是以总数代替 owner检查。

## 4. T0 / E0 / R0 follower blockers

### T0 — std/runtime transport shared RPC checkpoint

F440B 冻结的新 RPC surface 尚未开始实现；以下精确 symbol/file searches均为零：

- `requestJsonToConnection`
- `WebSocketRequestError`
- `ConnectionRequestRegistry`
- `connection.request`

当前最早可执行 blocker 是 transport shared corpus仍使用 gateway identity v1，导致必跑 transport suite
`79 passed / 4 failed`。这是 T0 transport wire接入所需输入；跨系统 corpus最终刷新仍由 F0 owner完成。
Cancellation direct probe `1/1` 通过，所以这不是 required cancellation checkpoint 回归。

### E0 — runtime typed inbound/outbound execution

`cargo check -p skiff-runtime-eval` 的三个首错精确为：

1. `runtime/eval/src/runtime_http_gateway.rs:85` 未覆盖
   `GatewayAdapterKind::WebSocketJsonRpc`；
2. `runtime/eval/src/runtime_http_gateway.rs:439` 未覆盖
   `GatewayAdapterSource::{WebSocketJsonRpcParams, WebSocketBusinessIdentity}`；
3. `runtime/eval/src/runtime_websocket_connect.rs:171` 未覆盖同两个 source。

Host check在 eval dependency同处停止，尚未暴露 Host 自身后续 match arms。
`RuntimeAssemblyWebSocketJsonRpcTarget` 与 `dispatchAssemblyWebSocketJsonRpc` 当前也均为零匹配。
`runtime/eval/.../service_error_channel/tests.rs:127` 的
`PlatformBuiltinErrorIdentity::Cancel` 是 test-only stale rejection fixture；production consumer为零。

### R0 — Router profile / broker / gateway hookup

Router 当前仍是明确的 follower：

- `router/src` 中旧 gateway v1 / deployment-artifact v2 reader或诊断共 `10` 行；
- `router/tests` 中对应旧 generation fixture共 `27` 行；
- `WebSocketJsonRpc|websocketJsonRpc` 在 `router/src router/tests` 中为 `ZERO_MATCHES`；
- `dispatchAssemblyWebSocketJsonRpc`、`connection.request` 也为 `ZERO_MATCHES`。

旧 reader精确位于 `runtimeAssemblySnapshot.ts`、
`runtimeAssemblyDeploymentSnapshot.ts`、`filesystemRuntimeAssemblySnapshotLoader.ts`、
`runtimeAssemblyRequest.ts`、`runtimeAssemblyRequestMetadata.ts` 与 `runtimeProtocol.ts`。
Router cancellation focused listing/execution仍为 `4/4 passed`，所以 R0 缺口只属于尚未实施的
JSON-RPC reader/profile/broker，不是 cancellation prerequisite 回归。

依赖顺序保持 F440B 冻结值：T0 可从本 P0 状态启动；E0 等待 T0；R0a 等待 T0 wire，R0b 等待 E0。

## 5. Scope 与禁令

- 只新增本文 result；未修改 production、test、fixture、权威设计或其它 task/result。
- 未访问 stable、live、instance、watch、Router/runtime端口或 service。
- 未安装依赖；Router只复用既有 dependency tree，临时链接已删除。
- 未派子 agent，未 merge、rebase、push。
- Result commit/tree由最终交付消息记录。
