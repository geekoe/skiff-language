# P5-F180F：Runtime Actor 实例存储与 Bootstrap 物化

状态：Ready

## 直接父任务

- `P5-F180D-actor-method-wire-error-contract-result.md`

## 目标

在 Runtime 内建立 Actor 实例的唯一存储 owner：按 `(logical Actor key, epoch)` 从 registry 保存的
bootstrap 与 linked Actor 声明物化字段 frame。该存储只管理实例身份、字段和生命周期 fence，
不向 registry 写回 live 字段。

## 范围

- Runtime host/eval 中新的 ActorInstanceStore 及必要 handoff；
- linked Actor declaration、boundary decoding 与 request heap 的复用；
- 聚焦单元/集成测试。

不得实现 Router admission、完整 Actor 方法执行器、协程 scheduler、升级 drain、崩溃/TTL 清理。

## 必须实现

- 实例键至少包含 canonical logical Actor key 和 epoch；ABI、implementation、declaration owner 是
  必须核验的实例 fence；
- 首次 activation 从 canonical bootstrap payload 按精确 linked Actor declaration 解码；
- bootstrap 字段必须与声明字段顺序、名称、类型和编码版本精确匹配；
- 同一 `(key, epoch, ABI, implementation)` 并发 activation 只物化一个实例；
- epoch、ABI、implementation 或 declaration owner 不一致不得复用；
- live 字段只保存在 owner Runtime 内存，不写回 Router registry 或 artifact；
- 提供后续 executor 获取受 fence 保护的实例/字段 frame 的接口，但普通 request、后台 task 或
  外部 capability 不能直接取得字段访问权限；
- 为 remove/replace/crash/idle 后续任务提供按 fence 丢弃实例的原子接口；
- 解码失败、声明缺失、字段错配、stale epoch 必须失败关闭且不留下半初始化实例。

## 验证

- 真实 linked Actor 声明 + canonical bootstrap 成功物化；
- 并发 activation 只产生一个实例；
- 字段缺失/多余/乱序/类型错、错误 owner/ABI/implementation/epoch、未知编码全部拒绝；
- 初始化失败不缓存，之后合法 activation 可成功；
- 丢弃接口只接受精确 fence，旧 fence 不影响新 incarnation；
- 证明 registry/bootstrap 原始记录不被 live 字段修改；
- Runtime 聚焦测试、`cargo check --workspace`、`git diff --check`；
- 独立提交并写 `P5-F180F-runtime-actor-instance-store-result.md`。

