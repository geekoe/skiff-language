# P5-F180H：Actor 挂起、让出与恢复 Fence 结果

状态：Complete

## 直接父任务

- `P5-F180G-actor-executor-self-fields-result.md`

## 已完成

- Actor 方法不再因 `maySuspend` 在执行前失败。
- 每个同步片段持有独占 execution lease；异步等待前：
  - 提交合法字段写入；
  - 使 execution token 失效；
  - 释放实例 scheduler；
  - continuation 的 fence 只保存普通 instance identity，等待期间 lease 容器为空。
- 恢复前重新进入同实例 scheduler，并通过 `ActorInstanceStore` 复核：
  - logical Actor key；
  - epoch/incarnation；
  - Actor ABI；
  - implementation；
  - materialized instance identity。
- 恢复不会替换 continuation 的原 request heap。最新 Actor 字段通过 linked field type plan
  从实例 heap 编码并解码到 continuation heap，因此挂起前普通局部 heap handle 保持有效。
- 已接入真实异步等待边界：
  - canonical service call；
  - remote interface 与 callback capability；
  - stream next；
  - stream emit/send；
  - DB operation/query/transaction/lease；
  - sleep、HTTP、file、Actor registry 等异步 native capability。
- stream next 先在当前同步片段内轮询一次：
  - 缓冲项已就绪时直接返回，不提交片段、不释放 scheduler；
  - 只有真实 `Pending` 才提交片段并让出。
- cancel/deadline 在重新排队阶段持续检查 execution budget；取消后不会重新安装 lease。
- stale epoch/incarnation/implementation 错误保留为 `ActorInstanceStoreError`，不会降级成普通
  artifact 字符串错误。
- `connection.send` 按用户决策保持同步：
  - 仅写入现有本地发送 channel；
  - 不让出 Actor 执行权；
  - 不增加 ack、backpressure 或 exactly-once 语义。

## 验证

- `cargo test -p skiff-runtime-eval --no-fail-fast`
  - 114/114 通过。
- `cargo test -p skiff-runtime-eval actor_executor --no-fail-fast`
  - 11/11 通过。
- `cargo check --workspace`
  - 通过。
- `git diff --check`
  - 通过。

聚焦探针覆盖：

- 写字段、挂起、另一方法更新、恢复读取最新字段；
- 等待期间无 active lease，字段访问失败关闭；
- continuation 普通局部 heap handle 跨恢复保持有效；
- stale epoch 恢复失败且不重新安装 lease；
- cancel 恢复失败且不重新安装 lease；
- async native route 精确产生调度切点；
- 已缓冲的 stream next 保持当前 execution lease，不产生调度切点；
- 四种 `connection.send` route 精确保持 `maySuspend=false`，不产生调度切点。

F180G 已有测试继续覆盖同实例同步片段不交错和不同实例并行。
