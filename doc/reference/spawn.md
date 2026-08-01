# Skiff Spawn Reference

本文负责：`spawn` 语句的用户可见规则——后台提交一次函数调用，让它独立于当前 request 执行。

本文不负责：Router 与 Runtime 的内部派发协议、观测字段。

## 定位

`spawn` 表达“提交这个调用，然后当前 request 继续往下走”。它是平台唯一的后台调用 surface，常见用途是唤醒一段后台推进，例如对某个业务对象 try-claim 并 drain。

```skiff
spawn runThreadDrain(threadId)
```

`spawn` 是 statement，不是 expression：它不产值，不能出现在表达式位置。

## Target 规则

- target 必须是当前 service 构建内的普通 function：service 自身的任意 module，或该 service 依赖的 package 中的 function（package 代码也可以 spawn 自己的函数）。跨 service callable 不能作为 target。
- target 也可以是当前 service 构建内的 actor 方法：`spawn actor.method(...)` 或同一 actor 实例内的
  `spawn self.method(...)`。提交的调用是该 actor 的一次独立方法调用，按 actor identity 路由，不嵌套在
  发起方调用栈内。
- target 返回类型必须是 `void` / `null`；返回值没有接收方。
- 参数必须是可恢复值。`spawn` payload 是 owner-internal recoverable boundary：plain data、可恢复 nominal object、
  durable native handle 和 `carrier = Local` 且 self payload 全可恢复的 `any I` 可以进入；callback、`Stream`、
  transaction、live connection、file descriptor、无 durable adapter 的 native handle、`carrier = Remote` 的 `any I` 等
  native/request-local resource fail closed。
- `spawn` 不允许出现在 `db transaction` 内。
- `spawn` 的目标方法必须在同一构建内可解析；actor 方法 receiver 必须是 actor 句柄（外部变量或 `self`）。
  `create` 内不允许 `spawn self.method(...)`。

## 执行语义

- payload encode 在提交前完成；若任一参数不可恢复，提交失败按普通平台错误抛给 caller，平台不得提交半截 work item。
- Router 收到提交后，直接在提交方所在的同一 Runtime replica 上创建一个新的内部 request。该 request
  精确钉死提交方当前 service、version、build 和 activation，不经过持久队列、claim 或后台 worker。
- 提交方只等待 Router 为新 request 建立普通 pending owner，并把现有 `request.start` 成功交给该
  Runtime 连接；随后 `spawn` 语句立即完成，不等待目标函数执行结果。没有匹配的同一 Runtime、
  Runtime 已关闭 admission，或该连接的`router.yml.runtime.maxConcurrency`统一pending容量已满时，提交
  失败。父request和所有direct-spawn derived request占用同一个连接级容量；没有spawn专用并发池、
  service级`maxConcurrency`或排队。
- `spawn` 是 same-build 执行语义：spawned call 必须由与提交方相同 service/version/build 的 runtime 执行。这个约束属于
  Router 对父 request 的认证和派生 request admission，不属于 recoverable args payload。
- args recoverable payload 不承载 `artifact_identity`、`build_id`、service version、package version 或 activation identity。
  `carrier = Local` 的 `any I` self payload 用当前 execution context + stable `LocalConcrete` restore key 恢复；spawn decode
  使用 target executable 的当前 expected type plan，policy 仍是 strict。payload schema 不一致、target expected type 不匹配或
  local concrete/projection 在 target executable 中不可用时，执行在 payload decode 阶段 fail closed；平台不从 payload 中读取
  历史 build/artifact 作为 fallback。
- 提交成功后，spawned call 与 caller request 生命周期分离；caller 后续 cancel / timeout 不影响它。
- spawned call 在新的、独立的 runtime request frame 中执行，不继承 caller 的 request-local 状态。
- actor 方法为 target 时，spawned call 不创建新的 service request frame：Router 把它作为该 actor 的一次
  普通方法调用，经 actor admission（含不 live 时按 registry entry 保存的创建输入激活）派发到 owner
  Runtime，在实例的单线程 executor 上与其他调用串行排队执行；调用方只等待“已接收”（Router 已完成
  admission 并派发到 owner）。spawned actor 方法拥有独立于 caller 的固定 deadline（120s），不继承
  caller 的剩余时间。
- spawned call 的完成或失败只结束该派生 request；失败不回传给已经完成的 `spawn` 语句，也不自动重试。
- 一次提交至多执行一次；执行失败、超时或 runtime 断连后，平台不自动重试同一次提交。
- spawned call 的业务结果必须自行落 DB / 事件 / 文件；平台只记录执行错误。
- actor 方法为 target 时，spawned 调用的结果/失败只结算 actor invocation ledger（并释放实例繁忙状态），
  不会回传给提交方；同一实例的多个 spawned 调用与普通方法调用一样串行执行。

测试运行可以附带平台签发的 opaque case capability，使派生 request 使用同一 case 的 inline-effect
registry。它只存在于测试控制面和 Runtime 内存中，不是业务可构造的 request-local 值，也不改变生产
`spawn` 不继承 request-local 状态的规则。测试 case 的 finalization 会等待已经被 Runtime 接受的派生
request 结束；取消测试根 request 仍不会取消这些派生 request。

## 可靠性边界

`spawn` 不是业务持久层。需要跨重启可靠推进的工作，必须先把业务事实写入 service-owned database，再用
`spawn` 做一次唤醒；Runtime 在接受派生 request 前失败时提交方会直接看到失败，接受后发生的 Runtime
崩溃仍可能丢失这次唤醒，必须由业务自己的恢复路径（扫描业务状态并重新 `spawn`）补偿。

重复 spawn 必须是安全的：执行体应通过 DB 状态（例如 lease try-claim）保证幂等，拿不到推进权时空跑退出。

`spawn` 参数和 DB/queue payload 使用同一可恢复值底线；差异只来自各自额外 policy。完整 contract 见
[`../architecture/recoverable-value.md`](../architecture/recoverable-value.md)。

## 当前不支持

- 返回值、callback、await handle。
- delay、retry policy、dedupe、priority、并发 key。
- 取消已提交的 spawned call。
