# P5-F180H：Actor 挂起、让出与恢复 Fence

状态：Ready

## 直接父任务

- `P5-F180G-actor-executor-self-fields-result.md`

## 目标

把 F180G 只支持同步片段的 Actor executor 扩展为协程执行：方法在异步边界挂起时提交当前同步片段、
释放实例执行权；恢复前重新取得执行权并复核 incarnation/epoch/implementation，使同实例其他方法
可在挂起期间推进字段。

## 范围

- runtime/eval continuation 与 Actor executor/scheduler；
- Host capability/stream/timer 的 Actor suspension handoff；
- 取消、deadline 与恢复错误映射；
- 聚焦并发测试。

不得实现 Router upgrade 状态转换、Runtime crash/TTL（已有独立任务）或 exactly-once。

## 必须实现

- 以下边界会让出实例执行权：
  - async service call；
  - stream next/顺序消费在下一项尚未就绪时的真实等待；
  - timer/sleep；
- `connection.send` 同步写入本地发送队列，不让出执行权；
- Runtime 不提供显式 `yield`，也不在同步指令之间自动抢占；
- 让出前提交当前同步片段的合法字段写入并使 execution token 失效；
- continuation 只能保存普通局部值和可重建执行位置，不得持有：
  - 裸字段引用；
  - F180G execution token；
  - scheduler guard；
  - 可绕过 heap/type plan 的字段内部句柄；
- 恢复时重新排队取得同实例 execution token，并精确复核：
  - logical Actor key；
  - epoch/incarnation；
  - Actor ABI；
  - implementation；
  - instance identity；
- 挂起期间另一方法可运行并修改字段；原方法恢复后再次读取 `self.field` 时看到最新值；
- stale epoch/incarnation/implementation 恢复返回
  `ActorIncarnationReplacedError` 或 `ActorVersionRejectedError`，不得继续执行；
- cancel/deadline 在挂起和排队恢复时都可终止 continuation，且不会泄漏 token、scheduler slot 或
  pending capability；
- 同步片段内部仍不交错；不同实例继续并行。

## 验证

- 方法写字段→挂起→另一方法改字段→恢复读取新值；
- 同实例同步片段不交错，挂起期间允许另一方法执行；
- 两个不同实例并行；
- service call、等待中的 stream next、timer 各有让出探针；
- 已缓冲的 stream next 与 `connection.send` 均有“不让出”的探针；
- continuation 不含 field/token/guard 的结构与行为负例；
- replace/remove/新 epoch 后 stale resume 精确失败；
- 挂起取消与 deadline 清理无泄漏；
- eval/host 聚焦测试、`cargo check --workspace`、`git diff --check`；
- 独立提交并写 `P5-F180H-actor-suspension-resume-fence-result.md`。
