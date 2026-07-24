# P5-F180K：Actor Owner Lease 与 Idle TTL

状态：Ready

## 直接父任务

- `P5-F180E-actor-router-admission-owner-state-machine-result.md`

## 目标

在 F180E 的原子 owner fence 上实现真实 lease 续租/过期和 Actor idle TTL sweeper。idle 逐出只丢弃
Runtime live 实例与 owner，不删除 registry entry 或 bootstrap；下一次调用应重新激活。

## 范围

- Router Actor owner lease renewal/expiry；
- Router idle TTL 状态与 sweeper；
- Runtime owner control handoff 中必要的逐出通知/确认；
- 聚焦时钟与并发测试。

不得实现 Actor 方法 executor、升级 drain、Runtime crash 恢复或修改 registry 的
getOrCreate/replace/remove 公共语义。

## 必须实现

- owner Runtime 使用完整 F180E fence 周期性续租；
- 只有 exact epoch/implementation/runtime/lease token 能续租；
- lease 到期后 Router 原子关闭旧 owner 接纳、失败其未完成 ledger 项并释放 owner；
- idle 时间基于真实 admitted/dispatched/completed ledger 活动推进，不能只看 registry 创建时间；
- 有 active invocation 时不得 idle 逐出；
- idle TTL 到期后 Router 向 exact owner 发送带 fence 的逐出控制，并在确认或 lease 到期后释放 owner；
- stale 通知/确认不能逐出新 incarnation 或新 owner；
- idle 逐出保留 logical registry entry、epoch、ABI、implementation 和原始 bootstrap；
- 下一次同 implementation 调用可重新 activation；
- 使用可注入 monotonic clock，测试不得依赖真实 sleep 或日期。

## 验证

- exact renewal 成功，错误 fence 全部拒绝；
- lease expiry 关闭接纳、失败 ledger、释放 owner；
- active invocation 阻止 idle eviction；
- 完成最后调用后从正确时间点计算 TTL；
- stale eviction ack 不影响新 owner/epoch；
- idle 后 registry/bootstrap 保留并可重激活；
- Router 类型检查、聚焦测试、`git diff --check`；
- 独立提交并写 `P5-F180K-actor-owner-lease-idle-ttl-result.md`。

