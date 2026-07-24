# P5-F180I：Actor 升级 Drain 与 Epoch 切换结果

状态：Completed

## 直接父任务

- `P5-F180E-actor-router-admission-owner-state-machine-result.md`
- `P5-F180H-actor-suspension-resume-fence-result.md`

## 已完成

- Router 为每次升级保存精确的旧 epoch、旧 implementation、owner Runtime、owner lease 和唯一目标
  implementation fence。
- 第一个不同 implementation 调用原子关闭新接纳；目标调用按各自 deadline 等待，同一 logical
  Actor 的后台升级不继承某个调用的 deadline，调用超时后升级仍继续。
- drain 只依赖 invocation ledger 中匹配完整旧 fence 的 active 集合；terminal transition、owner
  断连和 lease 失效通过事件通知等待者，不使用固定 sleep。
- 升级顺序固定为：
  1. 通知旧 Runtime 标记 exact incarnation upgrading；
  2. 等待旧调用 active 清零；
  3. 精确丢弃旧实例；
  4. 推进 registry epoch、清除旧 owner；
  5. 把原 registry bootstrap 交给目标 implementation Runtime 激活；
  6. 获取新 owner lease 并标记 live。
- Runtime upgrading fence 允许已经持有 execution lease 的同步片段正常完成和提交；方法到达真实
  挂起点后再次获取 scheduler 时返回 `InstanceReplaced`，不恢复旧 incarnation。
- Runtime 精确丢弃要求 Router session、完整 Actor fence、materialized instance identity 和
  upgrading 状态全部匹配。stale、伪造和重复通知均无副作用。
- 丢弃旧实例时 Runtime 将本地 epoch floor 推进到下一 epoch，并清除 session tracking；延迟到达的
  旧激活不能重建旧实例。
- 新 epoch 只从 registry 保存的 bootstrap 重新 materialize，不复制旧实例字段。
- Router 记录已退休 implementation；V1 升级到 V2 后，V1 调用返回
  `ActorVersionRejectedError`，不会触发降级。从未使用过的新 implementation 仍可成为下一次升级
  的唯一目标。
- 相同 implementation 的调用不触发升级；service version 不进入 Actor identity 或升级判断。
- Router 的 Runtime transport seam 和 RuntimeHost 的 exact upgrade API 已建立；真实跨进程 Actor
  method/control frame 接线由后续端到端任务统一闭合。

没有增加字段迁移、ABI 兼容推断、自动重试、显式 `yield` 或 exactly-once 语义；
`connection.send` 仍不让出执行权。

## 验证

- Router Actor admission、disconnect、owner lease/idle TTL 聚焦测试：15/15 通过。
- Router 类型检查：通过。
- Runtime Actor 聚焦测试：36/36 通过。
- Runtime Host 编译检查：通过。
- `cargo check --workspace`：通过。
- `git diff --check`：通过。

聚焦探针覆盖 active drain、目标调用等待与 deadline 超时后后台继续、严格的
mark → discard → epoch transition → bootstrap activation 顺序、stale completion、V1 防降级与新 V3
升级；以及同步片段完成、挂起恢复拒绝、exact/重复 Runtime 通知、旧 epoch 拒绝和新 bootstrap
字段重建。
