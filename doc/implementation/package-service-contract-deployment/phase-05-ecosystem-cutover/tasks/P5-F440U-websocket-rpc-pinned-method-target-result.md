# P5-F440U WebSocket RPC pinned method target / handlerless admission result

状态：`PASS / E0A_SCOPED_CHECKPOINT_VALID`。

本节点完成 E0 的 target/lifecycle 半边：

- `runtime/request` 新增 generation-pinned `RuntimeAssemblyWebSocketJsonRpcTarget`；
- Host loader 将一个 physical WebSocket entry 与同 deployment、同 candidate 的 sibling method
  entries 固化为 immutable table；
- method-bearing、无 connect handler 的 WebSocket 在 attach 前先取得 exact generation acquire
  receipt，再合成 default accept；
- path-only、无 handler、无 methods 的 Host request 保持 fail closed 且 zero acquire；
- method target 只从 connection pin 保存的 old `ActiveAssemblyRoute` 解析，不读取 current active
  assembly，也不做 artifact I/O。

本节点没有 decode params、执行 method handler、编码 result，也没有修改 runtime eval、Host eval
adapter、transport/wire DTO、Router、artifact/compiler schema。F440T 的
`WebSocketJsonRpc` wire variant 在 Host exhaustive match 中保持显式 `Unsupported`，真正 wire
execution 入口留给 E0b。

## 1. 基线与提交

| 状态 | Commit | Tree |
| --- | --- | --- |
| 任务声明的 integration baseline | `243ebc6b2ad0ca0b6327aed11ebbbbcd575365e2` | `5caa71e112b263ec0b835405862b94192b8e81b0` |
| worktree 实际起点 | `c4fb8586fc6f3ee082d4a2facf4803ab2ebdcdd4` | `0466853c4e6bcd634952eafaf88bf8fcd1a0cef9` |
| implementation | `17bc8282a38555f65bb87c3054dd47d819061f03` | `87024f462f7e3f4df2817cd1ed5b4484364fd88f` |

`c4fb8586` 直接基于 `243ebc6b`，中间只增加本任务文档，没有 production/test 变化。
Implementation 与本文 result 分离提交；result commit/tree 由最终交付消息记录。

Worktree：
`/Users/geek/workspace/skiff-p5-f440u-pinned-rpc-target`

Branch：
`codex/p5-f440u-pinned-rpc-target`

## 2. Test-first RED

最初加入 loader positive 后，F440T 新增的 closed request enum 先暴露
`assembly_wire.rs` 缺少 `WebSocketJsonRpc` exhaustive arm。按任务允许的最小机械分支补成显式
`Unsupported` 后，取得真实 runtime assertion RED：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-host \
  websocket_jsonrpc_target_admission_accepts_handlerless_physical_with_method_sibling \
  -- --nocapture
```

结果：实际执行 `1`，`1 failed`。旧 loader 精确报：

```text
declares more than one WebSocket ingress selector
```

该失败证明旧模型仍把 method sibling 当作第二条 WebSocket attach selector；不是零测试、测试 compile
error 或依赖遮罩。

后续 request target selector 首次编译时还暴露两个 baseline `#[cfg(test)]` fixture 未填写 current
S0-required `rpc_profiles`。经协调允许，仅在以下 tests-only constructors 增加
`rpc_profiles: Vec::new()`，没有 production logic 或测试语义变化：

- `runtime/request/src/http_gateway_target.rs`
- `runtime/request/src/websocket_connect_target.rs`

## 3. Immutable physical/method table

`ActiveAssemblyContextSet` 现在按 exact `ServiceDeploymentRef` 保存：

```text
physical WebSocket entry (method = None)
  -> optional connect handler
  -> immutable BTreeMap<method, sibling entry>
```

load/admission 对 physical 与 method sibling 执行以下 fail-closed join：

- deployment owner 与 activation owner 精确一致；
- exactly one compiler-owned physical `websocket` entry/binding；
- method selector 必须是同 host/path、`protocol=WebSocket` 的非空 `method=Some` sibling；
- physical/method owner、gateway key、canonical gateway identity、surface、adapter plan、
  linked entry pointer 与 ingress lookup 必须一致；
- method 必须有 real handler，且不得有 pre/guard；
- method profile 必须由 physical connect surface 明确支持；
- duplicate method、orphan method、额外 WebSocket selector、跨 owner、host/path/profile/identity
  drift 均拒绝。

method sibling 仍可被 linker 通过 selector 精确关联，但不会成为第二个
`method=None` attach route。

## 4. Pinned method target

`RuntimeAssemblyWebSocketJsonRpcTarget` 保存并复验 E0b 所需 exact facts：

- pinned `RuntimeAssemblyEvalTarget`、assembly identity/generation 与 activation owner；
- physical selector/key/identity/`WebSocketEntryId`；
- method selector、method gateway key/identity 与 profile；
- exact `Arc<LinkedGatewayEntry>`；
- implementation package build id；
- linked handler callable id、signature（含 formal/return type owner）与 executable address；
- canonical WebSocket JSON-RPC protocol surface及 adapter source plan。

构造时复验 canonical method identity、unary JSON-RPC surface、exactly one
`websocket.jsonRpcParams` source、formal/adapter parameter name集合、private implementation
callable target、package-local ABI signature与 execution image callable address。target 不持有
transport connection id，也不创建 runtime value。

## 5. Handlerless eager pin 与 exact receipt

generation registry 将 acquire 分为 tentative 与 acquired：

1. `begin_acquire_with_receipt` 保存 old physical `ActiveAssemblyRoute`，建立 tentative pin；
2. Host 先把 lifecycle acquire 发给 Router；
3. exact ACK 通过完整 lifecycle response/tuple 校验后才将 pin 标为 acquired，并完成 receipt；
4. mismatch、reject、send/encode rollback、release-before-ACK 或 session disconnect 均完成失败
   receipt并清理 tentative expectation；
5. sibling resolver拒绝任何没有 exact acquire receipt 的 pin。

connect admission 现在遵循：

- 有 connect handler：保持原 handler 执行语义；handler 返回 `Accept` 后先等待 exact acquire
  receipt，再发送原 accept；
- 无 connect handler且 method table 非空：先 acquire/receipt，再发送
  `Accept { business_identity: None, connection_policy: None }`；
- 无 connect handler且 method table 为空：Host request fail closed，不 acquire，不建立 pin；
- handlerless pin 的 close/release 使用同一 generation lifecycle owner，释放后 pin count 为零。

synthetic accept 只代表没有用户 connect callback；它不伪造 method handler result。

## 6. Old-generation sibling resolver

`WebSocketGenerationRegistry::websocket_jsonrpc_target` 的输入包含：

```text
router session
connection id
assembly identity/generation
physical WebSocketEntryId
host/path/method
method GatewayEntryIdentity
profile
```

resolver 先从 `(router session, connection id)` 的 acquired pin 取得保存的 physical
`ActiveAssemblyRoute`，再从该 route 所属 old `Arc<ActiveAssembly>` 的 immutable method table
精确查 sibling。它不调用 assembly controller/current snapshot，不重新 admit，也不读取 artifact。

测试先 pin generation A，再将 active replacement 为 generation B：

- A connection 仍返回 generation 1 target；
- B connection 返回 generation 2 target；
- A/B activation owners 为不同 `Arc`；
- wrong host/path/method、generation、physical id、method identity 均拒绝。

## 7. 规定 GREEN

所有 Cargo 命令统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 实际执行 | 结果 |
| --- | ---: | --- |
| `cargo test -p skiff-runtime-request websocket_jsonrpc_target` | 1 | PASS：1 passed / 34 filtered |
| `cargo test -p skiff-runtime-host websocket_jsonrpc_target --no-fail-fast` | 6 | PASS：6 passed / 291 filtered；3 integration binaries均 0 executed |
| `cargo test -p skiff-runtime-host websocket_generation --no-fail-fast` | 7 | PASS：7 passed / 290 filtered；3 integration binaries均 0 executed |
| fully-qualified required named test + `--exact` | 1 | PASS：1 passed / 296 filtered |
| `cargo check -p skiff-runtime-host` | — | PASS |
| `cargo fmt --all -- --check` | — | PASS |
| `git diff --check` | — | PASS |

fully-qualified selector 为：

```text
host::router_session::tests::websocket_generation_lifecycle::websocket_jsonrpc_target::handlerless_method_websocket_eager_pins_before_accept_without_user_connect
```

短名称配合 `--exact` 会执行零测试，因此未计为证据；上表记录的是完整路径的非零执行。

## 8. 补充 non-live 回归诊断

额外运行：

```text
cargo test -p skiff-runtime-host --lib --no-fail-fast
```

结果为 `291 passed / 6 failed`。六个失败均在 `c4fb8586` baseline 已存在的 stale identity test
literals 解析处 panic，未进入本 leaf production path：

- 4 个 assembly activation tests 仍写 `skiff-runtime-assembly-v1`，current parser要求 v2；
- `host_http_gateway_exact_route_identity_generation_mode_and_http_metadata_fail_closed` 使用 current
  parser拒绝的全 `f` gateway identity；
- `websocket_admission_rejects_gateway_identity_and_surface_mismatch` 使用 current parser拒绝的全 `0`
  gateway identity。

最后一个 stale literal 位于本 leaf 同时扩展的 colocated loader test 文件，但该语句可在
`git show c4fb8586:runtime/host/src/loader/active_assembly_context.rs` 中确认早已存在，且在调用
loader assertion前就 panic。按协调后的窄授权，本节点没有继续增加 tests-only S0 masks，也没有用
production compatibility 放宽 strict parser。所有任务规定 selector与 build/format gate均独立通过。

没有运行 live、server、stable instance、watch 或 chat smoke。

## 9. 自验收矩阵

| 任务条款 | 代码证据 | 测试证据 |
| --- | --- | --- |
| handlerless method-bearing attach先 pin后 accept | receipt-gated synthetic accept | required fully-qualified named test 1/1 |
| path-only/no-method zero acquire | admission rejects before queue | `websocket_jsonrpc_target_path_only_no_method_keeps_zero_acquire` |
| exact receipt mismatch attach前 cleanup | tentative/acquired state + oneshot failure | `websocket_generation_acquire_receipt_mismatch_fails_and_cleans_before_attach` |
| old A 不迁移到 active B | pin保存 old route/active Arc | A generation 1、B generation 2 target |
| exact tuple与method identity | acquired physical route + immutable sibling lookup | host/path/generation/physical/method identity negatives |
| duplicate/orphan/cross-owner/profile drift fail closed | loader physical/method table validation | 3 loader target admission tests |
| handlerless close/release归零 | shared lifecycle release path | required named test末尾 pin count 0 |
| target持有 E0b exact plans | linked entry/signature/addr/surface/adapter getters | request target constructor unit test |

## 10. 范围与反向审计

Implementation 提交共修改/新增 `11` 个文件，全部位于任务唯一写集：

- `runtime/request` target、export与两个已授权 tests-only S0 fixture masks；
- Host loader/admission、generation registry、connect admission/synthetic accept；
- 上述 Host 范围的 colocated fixtures/tests。

`assembly_wire.rs` 只增加 F440T closed enum 的 exhaustive request-id arm和显式
`Unsupported("RuntimeAssembly WebSocket JSON-RPC execution is not attached")`；没有把新 request
接到 eval、dispatcher或 response mapper。

没有修改：

- `runtime/eval`、JSON-RPC execution/outcome、Host eval adapter；
- `runtime/transport` 或任何 wire DTO；
- Router/gateway/server；
- artifact/compiler schema或 identity；
- 其它 task/result。

未派子 Agent；未 merge、rebase或 push。
