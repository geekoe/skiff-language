# P5-F180K：Actor Owner Lease 与 Idle TTL 结果

状态：Completed

## 直接父任务

- `P5-F180E-actor-router-admission-owner-state-machine-result.md`

## 结果

Router 已在 F180E 完整 owner fence 上实现 lease 到期与 idle TTL：

- `ActorOwnerLeaseIdleController` 只消费可注入单调时钟，不读取日期时间，也不依赖定时
  `sleep`；
- owner 续租必须精确匹配 logical Actor、epoch、implementation、Runtime 与 lease token；
- lease 到期 sweeper 会在同一个 store 临界区内关闭旧 owner 接纳、失败该 owner 的
  `admitted/dispatched` ledger 项并释放 owner；
- admitted、dispatched 和 terminal ledger 转换都会更新活动时间；最后一个未完成调用进入终态
  时才写入新的 idle 起点；
- active invocation 或既有 Actor execution 都会阻止 idle 逐出；
- TTL 到期先建立带完整 owner fence 和唯一请求 ID 的 `actor.owner.idle.evict` 控制消息，并立刻
  关闭该 owner 的新调用接纳；
- 只有完整匹配的 `actor.owner.idle.evict.ack` 才能释放 owner；旧 epoch、旧实现、旧 Runtime、
  旧 lease 或旧请求 ID 的确认均无效；
- 没有确认时 owner 保持被 fence，最终由 lease expiry 回收；
- idle 逐出与 lease expiry 都只清除 live owner，保留 logical registry entry、epoch、ABI、
  implementation、bootstrap encoding 与 bootstrap bytes；相同 implementation 可再次激活。

本任务没有实现 Actor 方法 executor、升级 drain、Runtime crash 恢复，也没有改变
`getOrCreate/replace/remove` 的公共语义。

## 验证

- `pnpm --dir router type-check`：PASS
- `pnpm --dir router exec vitest run tests/actor-owner-lease-idle-ttl.test.ts tests/actor-router-admission.test.ts tests/actor-manager.test.ts`：15/15 PASS
- Router 全量测试仍只有 F180E 已记录的 4 个既有失败：
  `spawn-queue.test.ts` 2 个、`assembly-runtime-endpoint.test.ts` 1 个、
  `compilerGeneratedManifestCompatibility.test.ts` 1 个；本任务新增和 Actor 聚焦测试全部通过
- `git diff --check`：PASS

测试使用手动推进的 fake monotonic clock，覆盖 exact renewal、错误 fence、lease expiry、
未完成 ledger 失败、active invocation 阻止逐出、最后完成时间作为 TTL 起点、stale ACK、
bootstrap 保留和重新激活；没有真实等待或日期依赖。
