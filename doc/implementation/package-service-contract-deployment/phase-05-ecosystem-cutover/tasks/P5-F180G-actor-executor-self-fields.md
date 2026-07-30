# P5-F180G：Actor 方法执行器与 Self 字段

状态：Ready

## 直接父任务

- `P5-F180F-runtime-actor-instance-store-result.md`

## 目标

把 F180D 的专用 Actor invocation handoff 接到 F180F 的实例存储和 F180B 的 linked Actor method
dispatch，建立单实例执行调度与 `self` 字段访问的唯一权限路径。

## 范围

- Runtime host Actor method handoff；
- runtime/eval Actor method executor；
- 单实例 scheduler；
- Actor `self` 字段读写 lowering/eval 所需的最小支持；
- 聚焦测试。

本任务只保证不发生挂起的同步执行片段串行。挂起时释放、continuation 与恢复 fence 属于 F180H。
不得实现 Router upgrade/crash/TTL。

## 必须实现

- 只接受已通过 Router admission 且携带完整 owner fence 的 F180D invocation；
- 精确解析 Actor declaration、ABI、implementation、method identity 和内部实现入口；
- 参数按 linked public method signature 解码，返回值按同一签名编码；
- Actor method 通过专用 `ActorDispatch` 执行，不能回退为普通 service request 或直接
  `ExecutableAddr` 调用；
- 为每个实例建立 scheduler：
  - 同实例同步执行片段绝不交错；
  - 不同实例可以并行；
- 每次获得实例执行权时产生不可伪造的 execution token；
- `self` 字段读写只能在 Actor method 当前 execution token 下进行；
- 普通函数、普通 request、后台 task、外部 capability 和另一个 Actor 实例不能访问该字段 frame；
- 写入必须沿字段 linked type plan 保持类型正确，并直接更新 F180F 的 live field frame；
- stale epoch、实例 identity、ABI、implementation、owner fence 在执行前再次校验；
- 当前遇到需要挂起的操作时必须返回明确的“协程尚未实现”状态，不得持锁跨 await 或悄悄串行等待。

## 验证

- 真实 Actor declaration/impl/invocation 执行并读写字段；
- 两个同实例同步方法并发调用不交错；
- 两个不同实例能由 barrier 证明并行；
- 普通/后台/外部上下文字段访问失败；
- 错误 method、参数、返回、owner/ABI/implementation/epoch 全部失败关闭；
- 字段写入类型错误不改变原值；
- 挂起点不持有实例锁并明确交给 F180H；
- eval/host 聚焦测试、`cargo check --workspace`、`git diff --check`；
- 独立提交并写 `P5-F180G-actor-executor-self-fields-result.md`。

