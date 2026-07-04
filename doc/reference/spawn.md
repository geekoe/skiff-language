# Skiff Spawn Reference

本文负责：`spawn` 语句的用户可见规则——后台提交一次函数调用，让它独立于当前 request 执行。

本文不负责：底层队列机制、router 派发实现、观测字段。

## 定位

`spawn` 表达“提交这个调用，然后当前 request 继续往下走”。它是平台唯一的后台调用 surface，常见用途是唤醒一段后台推进，例如对某个业务对象 try-claim 并 drain。

```skiff
spawn runThreadDrain(threadId)
```

`spawn` 是 statement，不是 expression：它不产值，不能出现在表达式位置。

## Target 规则

- target 必须是当前 service 构建内的普通 function：service 自身的任意 module，或该 service 依赖的 package 中的 function（package 代码也可以 spawn 自己的函数）。跨 service callable 不能作为 target。
- target 返回类型必须是 `void` / `null`；返回值没有接收方。
- 参数必须是可恢复值。`spawn` payload 是 owner-internal recoverable boundary：plain data、可恢复 nominal object、
  durable native handle 和 `carrier = Local` 且 self payload 全可恢复的 `any I` 可以进入；callback、`Stream`、
  transaction、live connection、file descriptor、无 durable adapter 的 native handle、`carrier = Remote` 的 `any I` 等
  native/request-local resource fail closed。
- `spawn` 不允许出现在 `db transaction` 内。

## 执行语义

- payload encode 在提交前完成；若任一参数不可恢复，提交失败按普通平台错误抛给 caller，平台不得提交半截 work item。
- 提交成功表示平台当前控制面已接受该调用。
- `spawn` 是 same-build 执行语义：spawned call 必须由与提交方相同 service/version/build 的 runtime 执行。这个约束属于
  submit / queue / claim 控制面元数据和 worker claim 过滤，不属于 recoverable args payload。
- args recoverable payload 不承载 `artifact_identity`、`build_id`、service version、package version 或 activation identity。
  `carrier = Local` 的 `any I` self payload 用当前 execution context + stable `LocalConcrete` restore key 恢复；spawn decode
  使用 target executable 的当前 expected type plan，policy 仍是 strict。
- 提交成功后，spawned call 与 caller request 生命周期分离；caller 后续 cancel / timeout 不影响它。
- spawned call 在新的、独立的 runtime request frame 中执行，不继承 caller 的 request-local 状态。
- 一次提交至多执行一次；执行失败、超时或 runtime 断连后，平台不自动重试同一次提交。
- spawned call 的业务结果必须自行落 DB / 事件 / 文件；平台只记录执行错误。

## 可靠性边界

`spawn` 不是业务持久层。需要跨重启可靠推进的工作，必须先把业务事实写入 service-owned database，再用 `spawn` 做一次唤醒；唤醒丢失时，由业务自己的恢复路径（扫描业务状态并重新 `spawn`）补偿。

重复 spawn 必须是安全的：执行体应通过 DB 状态（例如 lease try-claim）保证幂等，拿不到推进权时空跑退出。

`spawn` 参数和 DB/queue payload 使用同一可恢复值底线；差异只来自各自额外 policy。完整 contract 见
[`../architecture/recoverable-value.md`](../architecture/recoverable-value.md)。

## 当前不支持

- 返回值、callback、await handle。
- delay、retry policy、dedupe、priority、并发 key。
- 取消已提交的 spawned call。
- 以 actor method 作为 spawn target。
