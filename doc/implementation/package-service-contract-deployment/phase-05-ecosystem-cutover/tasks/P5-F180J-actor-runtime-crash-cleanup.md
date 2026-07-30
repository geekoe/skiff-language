# P5-F180J：Actor Runtime 断连与崩溃清理

状态：Ready

## 直接父任务

- `P5-F180E-actor-router-admission-owner-state-machine-result.md`
- `P5-F180F-runtime-actor-instance-store-result.md`

## 目标

当 owner Runtime 断连或崩溃时，用精确 owner fence 失败该 Runtime 上的 Actor 调用、释放 Router
owner，并丢弃 Runtime live 实例。registry entry 与原 bootstrap 保留，下一次调用在新 owner 上重新
激活。

## 范围

- Router runtime connection lifecycle 与 Actor dispatcher/ledger/store；
- Runtime host shutdown/disconnect hook；
- ActorInstanceStore 精确批量丢弃；
- 聚焦故障测试。

不得实现 exactly-once、自动重试、升级 drain、idle TTL 或方法 executor 语义。

## 必须实现

- Router 将 Runtime disconnect 映射到该连接当前持有的 exact Actor owner fences；
- 对每个 fence 原子执行：
  - 关闭新 admission；
  - 将 admitted/dispatched 未完成 invocation 标为 failed；
  - 释放 owner；
- stale disconnect 事件不得释放同 Runtime 重连后的新 lease，也不得影响其他 Runtime/epoch；
- Runtime shutdown hook 按完整实例 fence 丢弃 live instance；
- 丢弃失败或重复通知必须幂等且不能误删新 incarnation；
- registry logical entry、epoch、ABI、implementation 与 bootstrap 均保留；
- 下一次调用可由另一个 Runtime 从原 bootstrap 激活；
- 不自动重放失败调用，调用方只能观察失败并自行决定是否重试；
- 外部副作用可能已经发生的窗口必须在错误/测试中可观察，不能声称 exactly-once。

## 验证

- owner 在 admitted 前、dispatched 中、完成后断连的矩阵；
- 两个 Runtime 中一个断连不影响另一个；
- stale disconnect/重复 disconnect 不影响新 lease；
- live 字段被丢弃，registry/bootstrap 保留；
- 新 owner 从原 bootstrap 重建而不是继承崩溃前字段；
- 未完成调用精确失败且不会自动重试；
- Router/Runtime 聚焦测试、类型检查、`cargo check --workspace`、`git diff --check`；
- 独立提交并写 `P5-F180J-actor-runtime-crash-cleanup-result.md`。

