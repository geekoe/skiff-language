# P5-F440U WebSocket RPC pinned method target / handlerless admission

状态：Ready。E0a共享检查点；只建立generation-pinned method target与handlerless eager pin，不执行handler。

## 直接父节点

- `P5-F440T-inbound-runtime-assembly-websocket-rpc-wire-result.md`
- `P5-F440S-runtime-websocket-rpc-execution-preflight-result.md`
- `P5-F440B-bidirectional-websocket-owner-audit-result.md` §8.2
- `P5-F440Q-websocket-rpc-invocation-linkage-result.md`

F440T已冻结inbound request/response DTO；F440S证明E0应先关闭loader/admission与old-generation resolver，
再实现typed eval。F440B拥有pin与exact sibling route语义。

实现基线为`243ebc6b`对应的current integration tree。

## DAG位置与目标

本节点完成E0的target/lifecycle半边：

1. 在`runtime/request`建立`RuntimeAssemblyWebSocketJsonRpcTarget`（或current命名等价物），只持有执行所需
   的exact pinned route/handler metadata；
2. Host loader/admission把一个physical WebSocket entry与其sibling method entries建立immutable关联；
3. method-bearing entry即使没有connect handler，也在Router attach前走generation acquire并取得exact
   receipt，随后合成默认accept；
4. path-only且无connect handler/无methods的entry保持不触发runtime/pin；
5. method target只从connection generation pin持有的old `Arc<ActiveAssembly>`解析，不查询current
   active assembly；
6. exact校验deployment owner、host/path、protocol、method、physical `WebSocketEntryId`、
   method `GatewayEntryIdentity`、profile与assembly identity/generation。

本leaf不decode params、不执行handler、不编码result、不修改wire或Router。完成后解除E0b。

## 唯一写集

- `runtime/request/src/lib.rs`
- 新建`runtime/request/src/websocket_jsonrpc_target.rs`
- target的colocated tests
- `runtime/host/src/loader/{assembly_admission.rs,active_assembly_context.rs}`
- `runtime/host/src/host/websocket_generation.rs`
- `runtime/host/src/host/request_entry/{assembly.rs,assembly_wire.rs}`中admission/acquire/synthetic accept所需
  的最小机械分支
- 上述Host范围的colocated fixtures/tests
- 本leaf result

禁止修改runtime/eval、JSON-RPC execution/outcome、Host eval adapter、transport/wire、Router、
artifact/compiler schema、fixture/tooling、其它task/result。不得派子Agent，不得运行live/server。

## Immutable physical/method table

loader必须从同一个deployment owner内建立：

```text
physical WebSocket entry (method = None)
  -> optional connect handler
  -> immutable sibling method table (method = Some)
```

每个method sibling必须精确共享：

- deployment/package implementation owner；
- host/path；
- `protocol=WebSocket`；
- physical `WebSocketEntryId`；
- profile；
- active assembly candidate。

duplicate method、跨owner、不同physical id、orphan method、profile mismatch或一个path对应歧义physical entry均
在load/admission阶段fail closed。不得把method sibling当第二个可attach socket route。

`active_assembly_context`当前“只允许一个WebSocket binding”的假设应收敛为“一个physical entry及其
immutable sibling methods”，不能放宽为任意同path entries。

## Handlerless eager pin

connect admission规则：

- 有connect handler：保持现有handler语义，并在需要generation pin时先acquire；
- 无connect handler但method table非空：不得直接跳过runtime；先`begin_acquire`、等待exact
  generation receipt，再生成default accept；
- 无connect handler且method table为空：保持现有path-only fast path，不获取pin；
- acquire失败、receipt identity/generation/owner不匹配：attach前拒绝并释放expectation；
- close/release按generation owner执行，不依赖connect handler是否存在。

synthetic accept只表示“没有用户connect回调”；不得伪造handler result或绕过pin receipt。

## Pinned sibling resolver

resolver输入必须包含F440T request携带的exact tuple。查找只能从connection pin持有的active candidate及
physical route开始：

```text
(router session, connection id, assembly identity/generation,
 physical websocket entry id, method, method gateway identity, profile)
```

禁止：

- 调current assembly controller/snapshot重新admit；
- 仅按host/path/method全局搜索；
- assembly replacement后把old connection迁到new handler；
- 缺method identity时按字符串猜；
- transport connection id进入handler params/runtime value。

target至少携带后继E0b需要的exact linked callable、formal/return plan owner、adapter source plan与pinned
assembly metadata；不在本leaf执行。

## Test-first与验证

先增加以下至少一个真实red：

- loader当前拒绝handlerless method-only entry；
- connect当前在`handler===undefined`时未acquire就accept；
- resolver当前不存在或会读current assembly。

终态至少覆盖具名测试：

- `handlerless_method_websocket_eager_pins_before_accept_without_user_connect`；
- old generation A pin后active replacement为B，resolver仍返回A method target；
- current B resolver返回B，A/B target identity确实不同；
- path-only/no-method保持zero acquire；
- acquire/receipt mismatch在attach前fail并cleanup；
- cross-owner/host/path/profile/physical id/method identity mismatch拒绝；
- duplicate/orphan methods loader fail closed；
- close/release对handlerless pinned generation归零。

必跑：

```bash
cargo test -p skiff-runtime-request websocket_jsonrpc_target
cargo test -p skiff-runtime-host websocket_jsonrpc_target --no-fail-fast
cargo test -p skiff-runtime-host websocket_generation --no-fail-fast
cargo check -p skiff-runtime-host
cargo fmt --all -- --check
git diff --check
```

若实际selector名称不同，先list/确认非零执行并在result记录count。Cargo统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

## 停止与交付

若证明handlerless attach必须修改Router gateway，返回Host半边有效检查点与
`TASK_SCOPE_EXPANDED`，不得越界。若target需要执行器才能唯一拥有linked plans，保留exact route/handler
identity并在result记录E0b接口，不得提前实现eval。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440u-pinned-rpc-target`
- branch：`codex/p5-f440u-pinned-rpc-target`
- result：`P5-F440U-websocket-rpc-pinned-method-target-result.md`

Implementation与result分开提交；不merge/rebase/push。
