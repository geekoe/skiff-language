# P5-F180E：Actor Router 接纳与 Owner 状态机

状态：Ready

## 直接父任务

- `P5-F180D-actor-method-wire-error-contract-result.md`

## 目标

让 Router 按 F180D 的专用 Actor 方法帧，对 logical Actor、epoch、ABI、implementation 和 owner
进行原子接纳与路由。先修复 owner lease 可覆盖未过期 owner 的错误，再建立可供升级、崩溃和 TTL
任务继续扩展的状态机。

## 范围

- `router/src/actor/registryStore.ts`
- `router/src/actor/inMemoryRegistryStore.ts`
- `router/src/actor/manager.ts`
- `router/src/router/actorSpawnRuntimeControl.ts`，或从中拆出的专用 Actor method dispatcher
- Router 聚焦测试

不得实现 Runtime ActorInstanceStore、字段执行器、协程恢复、最终升级 drain/TTL sweeper。

## 必须实现

- 未过期 owner lease 不能被另一个 Runtime 覆盖；owner 取得、续租、释放必须带 epoch/lease fence
  并通过 store 原子操作完成；
- 建立 `inactive / activating / live / upgrading` 状态及合法转换；
- method admission 精确校验：
  - logical Actor key/ref；
  - expected epoch；
  - Actor ABI；
  - requested implementation；
  - method identity；
- 同 implementation 在 live owner 上复用；
- 不同 implementation 关闭新 admission，进入 upgrading，并返回 F180D 的精确 typed error；
- stale epoch 返回 `ActorIncarnationReplacedError`；
- 不可接受的旧/未知 implementation 返回 `ActorVersionRejectedError`；
- 建立 invocation execution ledger，至少精确记录 admitted、dispatched、completed/cancelled/failed，
  且每次转换带 invocation、epoch、implementation 和 owner fence；
- Actor method 只能发送给当前 fenced owner Runtime，不能走普通 service request；
- Runtime 断连后的完整恢复留给 F180J，但本任务必须暴露原子释放/失败 ledger 所需的状态机接口。

## 验证

- 两个 Runtime 并发抢 owner，最多一个成功；
- 未过期 lease 无法覆盖，错误 fence 无法续租/释放；
- 相同 implementation 复用 owner；
- 不同 implementation 关闭 admission 并进入 upgrading；
- stale epoch、错误 ABI、错误 implementation、错误 method 全部精确拒绝；
- invocation ledger 合法转换与重复/越序转换拒绝；
- 普通 service request 不能进入 Actor dispatcher；
- Router 类型检查、聚焦测试、`git diff --check`；
- 独立提交并写 `P5-F180E-actor-router-admission-owner-state-machine-result.md`。

