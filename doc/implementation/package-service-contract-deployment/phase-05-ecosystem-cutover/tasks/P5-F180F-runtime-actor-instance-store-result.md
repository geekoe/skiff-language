# P5-F180F：Runtime Actor 实例存储与 Bootstrap 物化结果

状态：Completed

## 直接父任务

- `P5-F180D-actor-method-wire-error-contract-result.md`

## 结果

已在 `skiff-runtime-eval` 建立 Runtime owner 内存中的 `ActorInstanceStore`：

- 唯一实例键为 canonical logical Actor key + epoch；
- logical key 会重新校验 Actor id encoding、canonical JSON bytes 和 SHA-256 hash 的一致性；
- Actor ABI、implementation identity 和 linked declaration owner 构成精确复用 fence；
- 首次 activation 只从精确 owner 对应的 `LinkedActorDeclaration` 解析字段；
- bootstrap 只接受 `skiff-canonical-v1`，字段 wire 顺序必须是 canonical key 顺序，字段集合必须与
  declaration 完全相同；
- 字段按 declaration 顺序写入实例 frame，并使用各字段的 linked type plan 和现有
  `RuntimeBoundaryCodec` 解码；
- 并发 activation 在原子发布前不会暴露半初始化实例，同一 fence 只产生一个实例；
- 新 epoch 物化后，旧 epoch 的 executor 字段访问会被 stale fence 拒绝；
- executor 字段入口和 authority 均限制在 eval crate 内，普通 host request、后台任务或外部
  capability 不能从公开 handle 读取字段；
- 丢弃接口同时核验完整 fence 和实际实例 identity，同 epoch 的旧清理 handle 也不能误删重新物化的
  新实例；
- live 字段、字段 heap 和 mutation 全部只保存在 owner Runtime 内存，不修改 registry bootstrap
  bytes、Router 状态或 artifact。

解码、linked declaration 缺失、owner/ABI/implementation 不一致、字段缺失/多余/乱序/类型错误、
未知 encoding 和 stale epoch 均失败关闭。失败路径不会写入实例表或推进最新 epoch。

本任务没有接入 Router admission，没有实现 Actor 方法执行器、协程、升级 drain、崩溃或 TTL 清理。

## 验证

- `cargo test -p skiff-runtime-eval actor_instance --lib`：13/13 PASS
- `cargo test -p skiff-runtime-eval --lib`：98/98 PASS
- `cargo check --workspace`：PASS
- `git diff --check`：PASS

聚焦测试覆盖真实 linked Actor declaration 物化、并发单实例、完整字段错误矩阵、owner/ABI/
implementation/epoch fence、失败后重试、精确丢弃、同 epoch 重物化清理竞态，以及 registry bootstrap
bytes 不受 live mutation 影响。
