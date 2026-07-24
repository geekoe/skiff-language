# P5-F180J：Actor Runtime 断连与崩溃清理结果

状态：Completed

## 直接父任务

- `P5-F180E-actor-router-admission-owner-state-machine-result.md`
- `P5-F180F-runtime-actor-instance-store-result.md`

## 结果

Router 与 Runtime 已建立按连接和实例完整围栏执行的 Actor 崩溃清理：

- Router 用 `runtimeId + sessionId` 记录连接实际持有的完整 Actor owner fence；
- Runtime 连接断开时，Router 在移除连接前取得精确连接代次，再逐 owner fence 原子失败
  `admitted / dispatched` 调用并释放 owner；
- owner 释放同时校验 logical Actor、epoch、实现身份、Runtime、lease 和 lease 到期时间；
- 已完成调用保持完成，其他 Runtime、其他 Actor、其他 epoch 和新连接取得的新 lease 不受影响；
- 旧连接迟到通知、重复断连和错误 Runtime/session 通知均幂等；
- registry entry、epoch、ABI、实现身份、bootstrap bytes 均不被清理，owner 释放后下一次调用可重新激活；
- 未完成调用的失败原因明确指出外部副作用可能已经发生，不承诺 exactly-once，也不自动重放；
- Runtime 的 `ActorInstanceSessionTracker` 将物化实例句柄唯一绑定到 Router session；
- session 退出时先原子取走该 session 的完整句柄集合，再按完整实例 fence 与实例指针身份批量丢弃；
- 重复或迟到清理不能删除同 epoch 后来重新物化的实例，进程 shutdown 会幂等清理全部已跟踪实例。

Router lifecycle 通过可选的 Actor 断连控制器接入；Actor owner transport 在取得 owner fence 时绑定
连接。Runtime Host 暴露最小实例跟踪入口，供 Actor executor 激活实例后登记。该任务没有实现方法
executor、upgrade drain、idle TTL、自动重试或 exactly-once。

## 验证

- Router Actor/endpoint 聚焦测试：27/27 PASS
- Router TypeScript 类型检查：PASS
- Runtime eval Actor 实例测试：15/15 PASS
- Runtime Host Router session 测试：31/31 PASS
- `cargo check --workspace`：PASS
- `git diff --check`：PASS

聚焦测试覆盖 owner 在 admitted、dispatched 和 completed 后断连；两个 Runtime 隔离；旧连接与重复
断连；新 lease 防误删；registry/bootstrap 保留；未完成调用精确失败且无自动重试；session live
字段丢弃；同 epoch 从原 bootstrap 重新物化；旧实例句柄不能跨 session 重新取得清理权限；shutdown
批量清理幂等。
