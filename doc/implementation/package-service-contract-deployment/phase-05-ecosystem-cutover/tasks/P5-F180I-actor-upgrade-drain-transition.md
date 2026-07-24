# P5-F180I：Actor 升级 Drain 与 Epoch 切换

状态：Ready

## 直接父任务

- `P5-F180E-actor-router-admission-owner-state-machine-result.md`
- `P5-F180H-actor-suspension-resume-fence-result.md`

## 目标

完成不同 Actor implementation 触发的升级：关闭旧 incarnation admission，等待已接纳同步片段返回或在
真实挂起点安全退出，销毁旧实例、推进 epoch，并在目标 implementation Runtime 上从原 bootstrap 激活。

## 范围

- Router upgrading 状态、invocation ledger、目标调用等待与 epoch transition；
- Runtime Actor executor 的 upgrading fence/安全退出；
- 旧实例丢弃与目标 bootstrap activation；
- typed error 与聚焦并发测试。

不得迁移旧实例内存字段，不得推断 ABI 兼容，不得增加 exactly-once 或自动重试。

## 必须实现

- 第一个不同 implementation 调用原子指定唯一目标 implementation 并关闭所有新 admission；
- 已 admitted/dispatched 的旧方法：
  - 无真实挂起点时允许运行到正常返回、失败或执行预算终止；
  - 到达真实挂起点后不再恢复旧 incarnation，返回 `ActorIncarnationReplacedError`；
- Router 以 invocation ledger 的 active 集合判断 drain 完成，不使用固定 sleep；
- active 清零后按 exact fence：
  - 通知旧 Runtime 丢弃实例；
  - 推进 registry epoch；
  - 清除旧 owner；
  - 在目标 Runtime 从原 bootstrap 激活；
- 目标触发调用可在自身 deadline 内等待切换；超时返回可重试的 `ActorUpgradingError`，但升级状态继续由
  Router 精确推进；
- 新 incarnation 激活后只接受目标 implementation；
- 旧 implementation 和第三种 implementation 返回 `ActorVersionRejectedError`；
- same implementation 跨 service version 继续复用，不触发升级；
- stale drain completion、旧 Runtime ack、重复 completion 不能推进或破坏新 epoch；
- 旧 live 字段不复制到新实例，新实例必须回到 registry bootstrap。

## 验证

- 同 implementation 跨 version 复用；
- 不同 implementation 关闭 admission、等待 active drain、推进 epoch并激活；
- 无挂起同步方法运行到返回，已挂起方法在恢复前退出；
- 目标调用成功等待和 deadline 超时矩阵；
- 旧/第三 implementation 精确拒绝；
- stale/重复 ack、completion、owner fence 不影响新 incarnation；
- 新实例从 bootstrap 重建且不继承旧字段；
- Router/Runtime 聚焦测试、类型检查、`cargo check --workspace`、`git diff --check`；
- 独立提交并写 `P5-F180I-actor-upgrade-drain-transition-result.md`。

