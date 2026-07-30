# P5-F440R Router WebSocket RPC profile / broker core

状态：Ready。对应F440B DAG的R0a；只实现Router内可独立单测的profile-neutral broker与
`jsonrpc-2.0-text` profile，不接gateway/RuntimeDispatcher。

## 直接父节点

- `P5-F440P-websocket-rpc-transport-checkpoint-result.md`
- `P5-F440B-bidirectional-websocket-owner-audit-result.md`
- `P5-F440O-bidirectional-rpc-prerequisite-gate-result.md`

F440P已冻结Rust/TypeScript `connection.request`、`connection.request.cancel`、
`connection.response` wire及`RuntimeEndpoint` captured runtime sender/session API。F440B §§5–7和§9
拥有broker/profile状态机、JSON-RPC classifier与race语义。本leaf只把这些既定语义落到Router core。

实现基线为`c2abd2e8`对应的当前integration tree。

## DAG位置与目标

R0a只依赖T0 wire，可与F440Q/E0并行。完成后提供：

1. profile-neutral `WebSocketRequestBroker`，以connection generation为隔离边界；
2. 独立outbound/runtime-request与inbound/peer-request两张active表及各自tombstone；
3. `jsonrpc-2.0-text`严格classifier/encoder；
4. runtime outbound request/cancel/response的完整core状态机；
5. peer inbound request/notification/cancel分类和可注入的fake dispatcher接口；
6. capacity、size、deadline、disconnect、duplicate、late terminal与exact-once write的直接单测。

本leaf不读取active assembly、不查runtime owner、不创建真实RuntimeDispatcher请求、不接HTTP/WebSocket
upgrade/server/gateway。R0b在E0后消费这里冻结的profile action与broker terminal API。

## 唯一写集

- 可新建：
  - `router/src/protocol/jsonRpc20TextProfile.ts`
  - `router/src/router/webSocketRequestBroker.ts`
  - 两者的直接supporting type/helper文件
- 上述新模块需要的最小export/index机械修改
- 可新建：
  - `router/tests/json-rpc-20-text-profile.test.ts`
  - `router/tests/websocket-request-broker.test.ts`
- `router/src/router/runtimeEndpoint.ts`只允许消费F440P已有callback/source API所需的最小类型接线；
  若可通过构造注入避免修改则不改
- 本leaf result

禁止修改F440P strict wire schema、`RuntimeDispatcher`、assembly/gateway/server/upgrade实现、
`assemblyControlPlane.ts`、README、Rust、fixture、scripts、其它task/result。不得运行真实server或网络。

任务Agent如把classifier与broker拆成写入互不重叠的两个明确子块，可以派至多一个有界子Agent；子Agent
不得再派Agent，父Agent必须统一接口、集成、测试和result。不得派开放式review或重复测试Agent。

## Profile-neutral broker

broker不得解析业务typed value或持有`RuntimeTypePlan`。它只持有：

- connection generation token及captured peer writer；
- captured origin runtime sender/session（outbound leg）；
- opaque profile action/id/method/params/result；
- capacity/deadline/cancellation token；
- active indexes、tombstone与唯一terminal token。

outbound和inbound必须是两张独立表；同值id可同时存在于两个方向。runtime correlation与peer
JSON-RPC id不能互相替代。connection replacement创建新generation；旧generation pending仍由旧writer/
captured runtime完成，不能迁到新socket。

tombstone按父结果冻结的有界TTL/容量/FIFO规则阻止duplicate与late terminal污染新request；驱逐后允许
peer重用id。所有settle先从active indexes detach并写tombstone，再调用外部writer/dispatcher terminal。

## `jsonrpc-2.0-text` profile

只接受UTF-8 text frame；binary frame属于未来profile并按当前规则拒绝。classifier必须严格区分：

- request：exact `"jsonrpc":"2.0"`、合法id、non-empty method、params仅object/array或缺省；
- notification：无id；交给inbound action但不建立response terminal；
- response success：exact id与`result`，`null`合法；
- response error：exact id与标准error object；remote code必须safe integer、message有界，可选data保持
  opaque JSON；
- cancel：按F440B冻结的JSON-RPC cancel method/params spelling，只取消同一generation的inbound id；
- invalid request/parse/method/shape、batch、unknown top-level字段与id类型按父结果精确分类。

不得把JSON number经JS `number`往返后改变id；允许的string/number/null id与canonical echo规则以F440B
为准。业务params/result/data保持opaque canonical JSON bytes或等价无损表示，不在Router做Skiff类型转换。

固定platform error code仅由profile映射broker action/terminal产生；remote错误不能冒充平台错误。

## Outbound状态机

`connection.request`：

- 验证captured runtime source、profile、connection ownership、method/payload/容量；
- 分配connection-lifetime不复用的peer request id；
- 安装active/deadline后写peer request；
- peer success/error只完成exact outbound key，并回原captured runtime session一条
  `connection.response`；
- runtime cancel只清outbound并best-effort发peer cancel，不回普通response；
- broker deadline清理后best-effort cancel并回`deadlineExceeded`；
-disconnect按generation批量settle`transportUnavailable`；
-protocol/size violation按父结果关闭peer并settle`protocolError`；
-late/duplicate response静默丢弃或按tombstone规则处理，绝不完成另一个request。

所有race最多一个terminal；pending/timer/lease/active indexes在direct tests结束均为0。

## Inbound core接口

R0a不实现真实runtime dispatch，但必须冻结R0b可消费的注入接口：

- request产生包含profile、connection id/generation、opaque id/method/params与execution token的action；
- notification产生无response terminal的action；
- cancel只作用exact inbound active token；
- injected dispatcher success/invalidParams/internalError/deadline映射为profile result/error；
- peer cancel/disconnect先settle/abort，late injected completion无write；
- duplicate active/tombstoned id按父结果close/cleanup；
- capacity、timeout、runtime unavailable等固定平台错误精确映射；
- transport peer id不得进入未来runtime payload/action的业务字段。

接口不得要求R0b修改wire outcome或artifact selector；若发现需要，按停止条件返回。

## 测试先行与验证

先用pure fake writer/dispatcher建立red，至少覆盖classifier当前不存在、同值双向id、duplicate、cancel vs
complete、deadline/disconnect之一。终态direct tests至少覆盖：

- classifier正负corpus：request/notification/result/error/cancel、batch/binary/malformed/unknown field；
- id无损、params/result/data opaque；
- outbound乱序、remote error、cancel、deadline、writer failure、runtime/peer disconnect；
- inboundrequest/notification/cancel、duplicate、capacity、late completion；
-同值双向id互不影响；
- generation replacement不迁移pending；
- tombstone eviction后id可复用，旧execution token不能完成新request；
- 每个race最多一次write，active/timer/lease归零。

必跑：

```bash
pnpm --dir router exec vitest list --root router \
  tests/json-rpc-20-text-profile.test.ts \
  tests/websocket-request-broker.test.ts
pnpm --dir router exec vitest run --root router \
  tests/json-rpc-20-text-profile.test.ts \
  tests/websocket-request-broker.test.ts
pnpm --dir router type-check
git diff --check
```

若pnpm wrapper不能正确传参，按F440P已记录方式调用现有vitest binary；必须先列出非零tests，并在result
记录实际命令/count。禁止展开完整Router suite。

## 停止与交付

若core必须修改RuntimeDispatcher、gateway/server/upgrade、T0 wire或artifact selector，返回
`TASK_SCOPE_EXPANDED`并保留仍有效的pure模块提交；不得越界。若F440B对id/cancel/profile spelling无法从
父结果唯一实现，返回`TASK_NOT_EXECUTABLE`并列出唯一决策问题。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440r-router-rpc-core`
- branch：`codex/p5-f440r-router-rpc-core`
- result：`P5-F440R-router-websocket-rpc-profile-broker-core-result.md`

Implementation与result分开提交；不merge/rebase/push，不运行live/stable/instance。
