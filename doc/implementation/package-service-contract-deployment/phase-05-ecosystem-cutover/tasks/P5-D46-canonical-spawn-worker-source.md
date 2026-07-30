# P5-D46：Canonical Spawn Direct Derived Request Authority

状态：设计已冻结，解除 canonical spawn 执行实现。

## 决策

`spawn` 不建立 worker、claim、lease、renew、专用队列或持久 work item。Router 收到
`spawn.submit` 后，直接创建一个新的内部 Runtime request：

- 目标固定为提交方当前 activation 内的精确 function executable；
- service、version、build、activation 和 Runtime replica 全部继承经过认证的父 request；
- Router 只在同一 Runtime WebSocket 上派发，不重新选择 replica，也不从 ambient registration
  猜 service/build；
- 新 request 复用现有 `request.start`、普通 pending owner、该Runtime连接的
  `router.yml.runtime.maxConcurrency`统一容量、active tracking、deadline、`response.end` /
  `response.error` terminal 和断连清理；
- Router 建立 pending owner并把 `request.start` 成功交给选定连接后返回 typed submitted receipt；
  不新增 spawn 专用 start/accepted frame，目标函数结果也不返回给父 request；
- 无匹配 Runtime、父 request 已终结、来源连接不匹配、admission 关闭或容量不足时，submit 直接失败；
- 接受后的父 request cancel/timeout 不取消派生 request；派生 request 失败不重试。

因此旧 `spawn.claim`、`spawn.renew`、`spawn.complete`、`spawn.fail`、spawn queue/store 和 Host
spawn-worker lifecycle 都应删除，而不是迁移到 RuntimeAssembly。

## 认证与内部协议

`callerRequestId` 只用于 Router 证明 `spawn.submit` 来自同一 WebSocket 上仍活动的父 request。
它不是 case identity，不能单独授权一个生产 caller 获得测试能力。Router 从已登记父 request 读取
精确 activation 与可选的测试 case capability，然后构造派生 request；Runtime 上行字段不能覆盖
这些 owner facts。

现有 `request.start` 增加一个严格的内部 invocation union 分支：

- gateway分支保持现有routing/HTTP/WebSocket字段不变；
- spawn分支使用同一RuntimeAssembly identity、generation和deployment routing facts，并携带
  `invocation = { kind: "spawn", targetKind: "function", target }`；
- binary payload仍只承载recoverable args；
- terminal继续使用普通`response.end` / `response.error`。

测试 case capability 是 Router 为 test dispatch 签发的随机 opaque token：

- 只进入 Router→Runtime 的内部 request wire和两端内存 registry；
- 不进入 service/package artifact、配置、API、recoverable args或任何持久格式；
- 派生 request 和递归 spawn 继承同一 token；
- Runtime 按 token 隔离并发 case，并让测试根 request 的 finalization 等待所有已接受派生 request
  终结；
- 根 request cancel/timeout不取消派生 request；成功、失败、取消、deadline、Runtime/Router断连和
  进程退出都必须释放对应引用，最后一个 owner 结束后清理 registry。

## Layer ownership

- Router：认证父 request、构造并派发 direct derived `request.start`、建立普通 pending owner并跟踪
  独立 terminal；不解释 target args。
- Runtime Host/request layer：按普通request entry校验派生 request pin与target，建立独立
  supervision scope并执行和上报terminal。
- Eval：按 exact executable 和 recoverable expected plan 解码 args并执行 function；不选择 route、
  replica或测试 capability。
- test-only Host context：持有 capability → inline-effect registry 和派生引用生命周期；不进入生产
  artifact或业务 surface。

本决策不新增 artifact schema、deployment projection或用户配置。

`runtime.maxConcurrency`只存在于Router实例配置，不进入Runtime bootstrap。HTTP unary/stream、
WebSocket connect/JSON-RPC、service call、package-test root和direct-spawn derived request共享每条
Runtime WebSocket连接的同一pending上限；Actor/control frame暂不计。Service profile中的
`lifecycle.maxConcurrency`必须删除，不能用spawn专用固定池代替。
