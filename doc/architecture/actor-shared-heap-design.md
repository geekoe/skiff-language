# Actor 实例共享 heap 求值设计

> Status: 设计稿 v4（权威设计），实现阶段使用 multi-agent-development.md 流程。
> 取代：[actor-instance-evaluator-design.md](actor-instance-evaluator-design.md)（canonical 字段表草案，保留为历史）。
> 相关文档：[actor-model.md](actor-model.md)、[runtime.md](../reference/runtime.md)（§6 Concurrent lane model）。

## 1. 决策演进摘要

v1 草案（canonical 字段表）经 review 否决：逐字段编码破坏跨字段别名身份；原地修改无法被赋值级
脏跟踪捕获；事务与并发块发布时机的假设与已测语义冲突。

v2 草案（共享 heap + 事务持有实例）经 review 判定“可行但按原样不可行”，三个核心问题：

1. 挂起漏斗并不持有 heap 借用——`&mut RequestHeap` 活在外层求值器 future 里，需要整个求值器状态机
   改为段作用域 guard（实际跨 await 载点约 130+，不是 30–50）；
2. 压缩不能靠现有 `HeapHandle` generation “fail closed”——新 arena 会复用 `(index, 0)`，旧句柄可静默
   指向新节点；
3. 事务 truncate 回滚不安全（前缀节点可指向后缀；error/env/字段根可逃逸到后缀），现状靠 live-roots
   rebase，不能发布裸 truncate。

v3 决策：

- 共享 heap 方向不变，按 review 修正求值器借用模型、压缩 epoch、失败段语义；
- 事务简化：v1 编译期禁止 actor 方法内与 `concurrent` lane 内的 `db transaction`（当前业务未使用）。
  普通 request 的事务原样保留，不在本设计范围内。

v3 独立 review 结论：“有条件可行，按现状不可直接实现”。关键修正（已并入下文各节）：

1. `HeapAccess::Shared` 必须携带 `Arc<Mutex<RequestHeap>>` 才能自行 reacquire（§4.1）；
2. §4.1 硬规则必须限定“共享 guard 不跨 `Pending`”，不得禁止普通 request 的 `Exclusive` 现状（§4.1）；
3. 跨 await 载点不止漏斗：`DbIrEvaluator`、timeout 子求值器、stream consumer 等嵌套 future 都要纳入（§4.2）；
4. provider-stream 路径可能把共享 arena 句柄带进 `tokio::spawn` 任务，需边界修复（§3.5、§7.2）；
5. 实例级 active/suspended 计数与压缩无现成实现；router 存在 idle 逐出与 upgrade 互卡竞态，需先修（§7.2）；
6. 事务禁令在“同包直接语法 + 本地 helper 可达性”可行；跨包 / service call 目标需要新的 artifact
   effect bit，v1 接受该边界为文档化限制（§5）；
7. arena epoch 由 arena wrapper 持有并在每次分配/解引用校验（§7.1）。

v4 决策（用户确认）：

- **`concurrent` 语言特性 v1 暂不支持（编译期拒绝）**：全仓搜索确认无生产/业务使用（skiff 仓库无任何
  `.skiff` 使用；internals 仅有注释/标识符/字符串中的 “concurrent” 字样；只有 syntax/compiler 内部测试
  使用该语法）。作为前置片删除运行时 concurrent 机制与 E3 bridge；
- 共享 heap 迁移按“切片”实施，第一批为：Slice 1（concurrent 暂缓，前置片）与 Slice 2（`HeapAccess`
  双模式 + actor 单路径 Shared 原型），见 §12。

## 2. 目标与非目标

目标：

1. 每 actor 实例一个共享 arena，字段为稳定根 slot；
2. 方法段级借用 arena：真实挂起前释放、恢复时重取（reacquire），实例内零复制、零编码；
3. 别名 live：挂起期间其他方法对共享节点的修改直接可见；
4. 普通 request 语义不变（独占 heap，不需要 reacquire），但求值器统一走模式通用 heap 访问。

非目标（v1）：

- 不做 canonical 字段表 / 字段版本化 / 惰性解码；
- 不引入 GC；arena 回收只靠 quiescence 压缩与实例逐出；
- 不引入 promise；
- **v1 不支持 `concurrent` / `serial`（编译期拒绝）**：删除运行时 concurrent 机制与 E3 bridge；
- actor 方法（含 `create`）内不支持 `db transaction`（编译期禁止；普通 request 不受影响）；
- 不改变跨 runtime 调用边界编解码（actor 调用仍走 router，与 service call 一致）；
- 不做硬线程亲和：actor 段保持当前 driver 线程 inline 执行模型（见 §8）。

## 3. 共享 arena 模型

### 3.1 存储与生命周期

- 实例 store 持有 `ActorInstanceState { fields: Vec<ActorFieldValue>, arena: SharedArena }`；
- `SharedArena = Arc<tokio::sync::Mutex<RequestHeap>>`；字段根是 arena 内节点句柄；
- arena 生命周期 = 实例生命周期：激活时创建（create 执行前写入 key 字段），逐出/升级时整体丢弃
  （按创建输入重建）。

### 3.2 段生命周期

1. 方法进入：acquire —— 等 admission（首次 create 完成后）→ lock arena → 校验 instance fence /
   epoch / identity / **arena epoch** → 取得段借用（guard）；
2. 同步段执行：字段读写、原地修改直接在共享 arena 上进行（无副本、无 wire 往返）；request-scoped
   校验在写路径执行（见 3.5）；
3. 真实挂起（唯一 yield 点）：释放 arena 借用，future 返回 `Pending`；
4. 恢复：reacquire（可能等待其他段）→ 重新校验 fence / identity / **arena epoch** → 继续；
5. 方法结束：归还借用；字段状态留在 arena。

### 3.3 与现状的对应（删除项）

- `ActorInstanceExecutionLease` 的 fields + heap 克隆 → 删除；
- `snapshot_persistent_fields` / resume 导入 → 删除；
- `write_field` wire 往返 → 删除（保留类型校验与 request-scoped 校验）；
- `commit_execution` 的字段 + heap 发布 → 退化为借用归还 + generation/epoch 校验；
- `ActorExecutionFrame::suspend` / `resume` → drop / reacquire + fence 校验；
- actor 路径的 `with_transaction_live_fields` / 事务回滚接入 → 删除（actor 内事务已禁止）。

### 3.4 别名与失败段语义

- 别名 live：字段↔字段、局部↔字段、参数/返回值共享；挂起期间其他方法的修改直接可见；
  actor-model.md 的“恢复后必须重新读字段”改为“别名 live，读取即当前值”；
- **失败段不保证字段原子性**：uncatchable 错误时，段内已执行的字段/节点修改保留（与“已提交副作用
  不回滚”一致）；不再复制“失败段不发布”的现状保证。相关测试改写为文档化行为；
- 编译器对“挂起前读取字段、恢复后依赖旧值”的诊断保留为提示性。

### 3.5 request-scoped 校验

- 现状在挂起/结束时全图扫描拒绝 callback capability、request-local exception、stream 字段；
  共享 heap 下坏句柄写入后立刻可见，校验必须前移到**写路径**（字段赋值 / 原地修改入口）与
  **压缩根收集**两个时点；
- 跨任务边界（spawn 载荷、stream producer）仍要求可编码值，不得携带共享 arena 句柄
  （沿用现有边界 codec 与 request-scoped 规则）。

## 4. EvalContext 借用改造（reacquire）

### 4.1 硬规则

- **任何能返回 `Pending` 的 future 状态中不得存在共享 arena 派生的 `&mut RequestHeap` 或共享 guard**
  （普通 request 的 `Exclusive(&mut RequestHeap)` 保持现状，不受此规则约束）；
- heap 访问一律通过 `HeapAccess`：
  - `HeapAccess::Exclusive(&'a mut RequestHeap)`：普通 request / 未共享路径，保持现状借用语义；
  - `HeapAccess::Shared { arena: Arc<tokio::sync::Mutex<RequestHeap>>, guard: Option<OwnedMutexGuard<RequestHeap>> }`：
    actor 实例 arena；段内持有 guard，真实挂起前 drop，恢复时由 `arena.lock_owned().await` 重取；
- `EvalContext.heap` 从 `&'a mut RequestHeap` 改为 `HeapAccess`，所有 heap 访问走 `heap_mut()`；
- `heap_mut()` 定义在 `HeapAccess` 上（字段级访问），不得定义在 `EvalContext` 上——否则会同时借用
  `self.env` / `self.context`，破坏约 40 处同语句双借点；
- 同步函数（deep clone、codec、materialize、error promote）继续收 `&mut RequestHeap`，不变。

### 4.2 改造面

- 跨 await 载点：`eval_context/actual_pending.rs::await_operation`、`program_db/wait.rs::await_operation`、
  `program_stream/current_scope.rs::next_with_actor`、`callback_native/prepared.rs` wait、
  actor frame `await_if_pending` / `resume`，以及 `program_execution.rs` 的全部 `Interpreter` async 入口；
- 实际载点约 130+（EvalContext 方法、Interpreter 入口、DbIrEvaluator、program stream、timeout 子求值器、
  assembly / callback / provider 分发、spawn ops）；**嵌套 future 是结构性改造，不是漏斗机械修改**；
- 普通 request 走 `Exclusive` 模式，语义与所有权不变，但代码路径统一经过 `HeapAccess`；
  这是全 API 面的机械改动，不是局部漏斗改动。

### 4.3 纪律与风险

- 释放 / 重取只允许发生在漏斗内；外层 eval 链只借 `HeapAccess`；
- Rust 借用检查拦截不了“guard 字段跨 await 存活”的逻辑错误（能编译，会造成实例串行化或死锁）；
  补充手段：debug 断言（挂起返回 `Pending` 前 guard 必须为 None）+ 每个漏斗路径的纪律测试；
- 风险控制：先在 actor 单路径（get → method → suspend → resume）做最小原型（Slice 2），验证
  `HeapAccess::Shared` 模式后再铺开。

## 5. 事务：v1 简化决策

### 5.1 规则

- **actor 方法内（含 `create`）禁止 `db transaction`**，编译期报错；
- 单一 db 操作（`db require` / `insert` / `upsert` / `update` 等）在 actor 方法内继续允许，
  不受影响；其外部副作用语义不变；
- 普通 request 的 `db transaction` 完全不变（现有 truncate + live-roots rebase 路径保留）；
- **禁令边界（v1 文档化限制）**：同包直接语法与本地 helper 可达性（经 `callable_effect_profiles`
  的 `db:transaction` access tag）编译期拒绝；跨包 / service call / interface 目标体内的事务当前
  不可见，v1 不承诺覆盖（如需覆盖，需新增 artifact effect bit，列为后续设计）。

### 5.2 理由

- 当前业务未使用 actor 内事务；该禁令直接移除事务锁、truncate 安全性论证、竞争者写入语义变更、
  事务 drop / cancel 清理等一整套复杂度；
- 未来需要时再设计：候选方向为“事务持有实例 + live-roots rebase（复用现状机制）”或
  “staging heap + 提交合并”，届时单独评审，不阻塞 v1。

### 5.3 影响

- `program_db` rollback 的 actor 接入点（`with_transaction_live_fields`）删除或退化为普通路径；
- 现有 actor 事务测试改写为“编译期拒绝”测试；普通 request 事务测试不变（并需补普通 request 专用
  事务回归测试，因为现有 runtime 事务测试全部是 actor 上下文的）。

## 6. Concurrent：v1 暂缓（前置片）

### 6.1 决策与证据

- v1 编译期拒绝 `concurrent` 语句、`concurrent value` 表达式与 `serial`（`serial` 仅合法于
  concurrent surface 内，随 concurrent 一并拒绝）；
- 证据：skiff 仓库无任何 `.skiff` 文件使用该语法；internals 仓库仅有注释 / 标识符 / 字符串中的
  “concurrent” 字样；唯一使用点是 syntax/compiler 内部测试（parser、lowering、execution_semantics）；
- 该决策删除共享 heap 迁移中的“单执行单元 concurrent”全部需求：无新 IR / 段表 / env write-set /
  无 E3 bridge；E3 与 concurrent 运行时机制随 Slice 1 删除。

### 6.2 删除范围（Slice 1）

- compiler：execution_semantics 对 concurrent/serial 拒绝；相关 lowering / 测试更新；
- runtime：`env/concurrent_scheduler.rs`、`env/lane_state.rs`、`eval_context/concurrent.rs`、
  `actor_executor/actor_concurrent_continuation/bridge.rs` 与 lane 子机制、相关测试删除；
- 保留：`ActorExecutionFrame` 本体（actor 挂起/恢复核心，Slice 2 重写）；`with_actor_execution_frame`
  上下文接入点；
- 文档：`doc/reference/runtime.md` §6 标注 v1 暂不支持。

### 6.3 未来路径（不阻塞 v1）

- 若恢复 concurrent：重新设计“单执行单元 + 静态段表”IR，并明确块级 yield / 错误语义；届时再评审。

## 7. 内存与回收

### 7.1 arena epoch（新机制）

- arena epoch 由 **arena wrapper 持有**（不在 `RequestHeap` 内部），注入每次分配产生的 `HeapHandle`；
  `RequestHeap::new()` 不得重置为 0；实例 arena 每次压缩/替换 bump epoch；
- 段借用、恢复、每次字段解引用校验 epoch；旧句柄 fail closed；
- 校验点在 `slot` / `slot_mut`（所有解引用已收敛于此）；`runtime_values_equal` 的 handle 相等快路径
  在 epoch 进入 handle 后自然安全（新旧 epoch 不相等）；
- 压缩时无活跃续体（见 7.2），旧句柄不可达；epoch 是防御性兜底，不是可达性证明。

### 7.2 quiescence 压缩

- 实例维护 active / suspended 续体计数（含 `create`；事务已删除故不参与）与 upgrade / discard 状态；
- 触发条件：计数 == 0、无 pending upgrade / discard、arena 规模超过阈值、实例仍存活；
- 操作：锁内克隆 live 根（字段）到新 arena → bump arena epoch → 原子替换 store arena；
- 升级 / 逐出优先级高于压缩：实例进入 `upgrading` 或被 discard 时跳过压缩；
- 跨任务边界（spawn 载荷、stream producer）不得持有共享 arena 句柄（§3.5 校验保证），
  因此 detached task 不构成压缩阻碍；
- **前置修复（独立于压缩）**：router `inMemoryRegistryStore` 存在 idle 逐出与 upgrade 互卡竞态
  （进入 `upgrading` 时未取消 pending eviction，逐出 ACK 清 owner 后 `upgradeFence` 永久等待）；
  压缩/升级优先级成立前必须先修复该状态机并补 router 回归测试。

### 7.3 provider-stream 边界（Slice 2 前置）

- `async_stream_cancel` 的 provider-stream 路径直接 `context.env.clone()` 后 `tokio::spawn`，
  共享 arena 句柄可能随 self carrier 进入 detached task；改为与 unary 路径一致的 detach-only env
  构造（或任务边界深拷贝全部 env 根），并补回归测试（共享 arena 句柄不得出现在 spawned task 中）。

### 7.4 限制

- per-instance arena limits（节点数 / 字节）：超限报平台错误（触发压缩或逐出留作后续策略）；
- 长方法（如 LLM 流式 turn）在运行期间独占 arena 并累积节点，压缩只能等其结束；由 limits 兜底，
  超限行为与现状 heap 限制一致；
- 与 no-GC 决策一致：不回收单对象，只做整体整理与整体销毁。

## 8. 线程模型

- Rust 线程 = OS 线程；tokio 为 multi-thread runtime；
- 现状：session-owned child work（actor owner invoke / control、request leases）由 runtime
  driver 线程 inline poll → 同一实例方法实际在同一（driver）线程执行；v1 **保持该模型**，
  不把 actor 段 `tokio::spawn` 到 worker 池；
- 因此不依赖“tokio 软亲和”作为机制承诺；arena 由互斥 / 借用串行化；
- 十万级挂起 actor / 十几线程规模下，线程迁移与缓存亲和收益可忽略；性能预算放在零复制、
  有界内存、短唤醒路径；
- 未来若需跨核并行：改为多 shard 单线程 executor（每 shard 托管一组实例）或专用调度设计，
  不改变共享 arena 模型。

## 9. 跨 runtime 边界与 identity

- caller ↔ owner 编解码不变（service call 语义）；owner 内部共享 arena 对调用方透明；
- 升级 / 逐出 / 断连：按创建输入重建，字段不跨任期；升级期间旧实例需无活跃段（计数 == 0）；
- ABI：字段存储形态是内部实现，不改变 artifact ABI；
- **implementation identity**：字段存储与求值器改造会改变规范化编译结果；必须重新冻结 identity
  测试向量，并回归 actor-model.md 的四个消费视图：同包直接调用、公共跨包调用、
  `kind: test` / `topLevelAlias` 测试视图、router 控制面（get / spawn / owner 路由）。

## 10. 验收矩阵

1. `concurrent` / `serial` 编译期拒绝；运行时 concurrent 机制与 E3 全部删除，无残留引用；
2. Slice 2 完成后：actor 单路径（get → method → 真实挂起 → resume）走 `HeapAccess::Shared`，
   挂起零复制零编码；guard 不跨 `Pending` 存活；
3. 普通 request 走 `Exclusive` 路径，行为与现有测试不回归；
4. `db transaction` 在 actor 方法内编译期拒绝（同包边界）；普通 request 事务测试不回归；
5. 压缩：epoch 失效无泄漏；计数 == 0 才触发；upgrade / 逐出优先（router 竞态先修）；
6. 失败段语义按 §3.4（不保证字段原子性）文档化并测试；
7. 升级 / 逐出 / 断连语义不回归；implementation identity 重新冻结；跨包视图 1–4 不回归。

## 11. 风险与开放问题

- R1. `HeapAccess` 双模式改造的真实函数面（~130+ 载点）是否可穷举；`Exclusive` 模式是否真正做到
  语义不变（Slice 2 原型验证）。
- R2. `db transaction` 同包禁令的边界：`create`、本地 helper 可达性、`spawn` 目标排除；跨包边界
  文档化限制是否可接受。
- R3. arena epoch 的表示与校验点（句柄结构变更的波及面：codec、序列化、测试向量）。
- R4. per-instance arena limits 与长方法增长的阈值；压缩触发条件（Slice 3+）。
- R5. 别名 live 与失败段部分写入对现有测试 / 文档的影响范围（Slice 2 测试矩阵）。
- R6. 失败段语义的两个候选（接受部分写入 vs 失败即丢弃实例）——本文选择接受部分写入，
  如 review 有异议需在评审中给出替代论证。

## 12. 第一批切片定义（multi-agent-development.md 执行依据）

### Slice 1（已完成并合入）：concurrent / serial 暂缓

- 目标：`concurrent` 语句、`concurrent value` 表达式、`serial` 编译期拒绝；删除运行时 concurrent
  机制与 E3 bridge 及关联测试；`ActorExecutionFrame` 本体保留（Slice 2 重写）。
- 写范围：compiler（execution_semantics、相关 lowering/测试）、runtime/eval（concurrent 相关模块与
  测试）、`doc/reference/runtime.md` §6 标注。
- 验收：编译拒绝正例/负例测试；`skiff-runtime-eval` 与 `skiff-compiler-source`（及相关 crate）聚焦
  测试通过；`rg` 证明无残留 concurrent 运行时引用（test-only 允许的除外）。

### 批次 DAG（接口冻结后并行，见 §13）

- **F1（在途）**：`db transaction` 同包禁令（compiler execution_semantics + 本地 helper 可达性）+ 
  移除 runtime actor 事务回滚路径（`with_transaction_live_fields`、rollback actor 分支）+ 
  删除 actor 事务测试、补普通 request 事务回归测试。
- **Wave 1（并行）**：
  - **F2（求值器核心）**：`HeapAccess` 双模式（§13.1）+ `EvalContext.heap` 改造 + 漏斗
    release/reacquire（§13.3）+ `Interpreter` 入口签名（§13.4）+ provider-stream 边界修复 +
    关联测试；
  - **F5（router，独立）**：`inMemoryRegistryStore` idle 逐出与 upgrade 互卡竞态修复 + 回归测试。
- **Wave 2（F1 + F2 合流后）**：
  - **F3（actor 层 + model）**：共享 arena store / frame / executor 重写（§13.5）+ arena epoch
    （§13.6）+ active/suspended 计数 + per-instance limits + quiescence 压缩 + 失败段部分写入
    语义测试。基线包含 F1（事务路径已移除）与 F2（HeapAccess API 已存在）。
- **批末**：集成 Agent 将 `integration/actor-shared-heap` 合入 `main` 一次。

## 13. 并行实现接口契约（冻结）

本契约在 Wave 1 前冻结。F2/F5 立即并行；F3 在 F1+F2 合流后启动，按本节接口编码，
不依赖 F2 的内部实现细节。

### 13.1 `HeapAccess`（新文件 `runtime/eval/src/heap_access.rs`，F2 拥有）

```rust
pub(crate) enum HeapAccess<'a> {
    Exclusive(&'a mut RequestHeap),
    Shared {
        arena: Arc<tokio::sync::Mutex<RequestHeap>>,
        guard: Option<tokio::sync::OwnedMutexGuard<RequestHeap>>,
    },
}

impl HeapAccess<'_> {
    pub fn heap_mut(&mut self) -> &mut RequestHeap;   // Shared: guard 必须 Some，否则 invariant 错误
    pub fn release(&mut self);                        // Shared: guard.take() 并 drop；Exclusive: no-op
    pub async fn reacquire(&mut self);                // Shared: guard = Some(arena.lock_owned().await)；Exclusive: no-op
    pub fn is_shared(&self) -> bool;
}
impl Deref / DerefMut for HeapAccess（Shared 经 guard；Exclusive 直接）
```

- 普通 request / 未共享路径 = `Exclusive`，语义与现状完全一致，release/reacquire 为 no-op；
- actor 实例 arena = `Shared`；guard 不得跨 `Pending` 存活；release/reacquire 只发生在漏斗内。

### 13.2 `EvalContext`（F2）

- `heap: &'a mut RequestHeap` 改为 `heap: HeapAccess<'a>`；
- 所有内部 heap 访问改 `self.heap.heap_mut()`；`heap_mut()` 定义在 `HeapAccess` 上，不在
  `EvalContext` 上（避免同语句借用 `self.env` / `self.context`）；
- 共享模式下，任何能返回 `Pending` 的路径不得持有 guard。

### 13.3 漏斗契约（F2 实现；F3 只经 `ActorExecutionFrame` 使用，不直接依赖漏斗签名）

- `actual_pending::await_operation`、`program_db::wait::await_operation`、
  `program_stream::current_scope::next_with_actor`、`callback_native::prepared` 的 wait：
  对 `Shared` 执行 poll-once（`Ready` 不释放；`Pending` → `release()` → await → `reacquire().await`），
  对 `Exclusive` 保持现状直接 await；
- `ActorExecutionFrame::await_if_pending` 语义（F3）：poll-once；`Pending` →
  `access.release()` → await future → `access.reacquire().await` → 校验 instance fence / arena epoch；
  F3 通过 `HeapAccess` 的公开方法实现，不依赖 F2 漏斗内部。

### 13.4 `Interpreter` 入口（F2）

- `call_program_executable*` 系列的 heap 参数由 `&mut RequestHeap` 改为 `&mut HeapAccess`（或等效）；
- 普通 request 调用点传 `Exclusive`；actor 调用点传 `Shared`；
- 同步函数（deep clone、codec、materialize、error promote）继续收 `&mut RequestHeap`。

### 13.5 Actor store 契约（F3）

- `ActorInstanceState { fields: Vec<ActorFieldValue>, arena: SharedArena }`，字段根指向 arena 节点；
- `acquire_segment(handle) -> SegmentLease`（含 guard + fence/epoch 快照）；release/commit 无复制；
- active / suspended 续体计数（create、段、恢复中、放弃、提交）；升级 / 逐出要求计数 == 0；
- per-instance arena limits；`compact_if_quiescent()`（计数 == 0 且无 upgrade/discard 时触发）；
- 失败段不保证字段原子性（§3.4）。

### 13.6 Arena epoch（runtime/model，F3）

- `RequestHeap` 增加 epoch（默认 0；`new_with_epoch(u32)`；`epoch()`）；
- `HeapHandle` 增加 epoch；`slot()` / `slot_mut()` 校验 handle.epoch == heap.epoch；
- `alloc_*` 以当前 heap epoch 盖章；压缩创建新 arena 时 epoch + 1；
- `runtime_values_equal` 的 handle 相等快路径因 epoch 入 handle 而安全。

### 13.7 Router（F5，独立）

- `router/src/actor/inMemoryRegistryStore.ts`：进入 `upgrading` 时取消/清理 pending idle eviction；
  upgrade 完成容忍 owner 丢失；补 router 回归测试。无跨模块接口依赖。

### 13.8 文件所有权（并行写集，互不重叠）

- F2：`heap_access.rs`（新）、`eval_context.rs`、`eval_context/actual_pending.rs`、
  `eval_context/timeout.rs`、`program_db/wait.rs`、`program_stream/current_scope.rs`、
  `program_stream.rs`（如需）、`callback_native/prepared.rs`、`program_execution.rs`、
  `db_eval.rs`（如需）、`spawn_ops.rs`（如需）、`async_stream_cancel.rs` +
  `prepared_unary.rs`（provider 边界）及关联测试；
- F3：`runtime/model/src/value.rs`、`runtime/model/src/request_heap.rs` 及测试；
  `actor_instance.rs`、`actor_executor.rs`、`actor_concurrent_continuation.rs` 及关联测试
  （基线含 F1，事务路径已移除）；
- F5：`router/src/actor/inMemoryRegistryStore.ts` 及 router 相关测试；
- F1（在途）：compiler execution_semantics、`program_db/rollback.rs`、`program_db/tests/transaction.rs`。

### 13.9 集成与验收

- 集成 Agent 串行合入 F1/F2/F5/F3 到 `integration/actor-shared-heap`；
- F2 与 F3 各自合入后必须先通过合并 HEAD 的 `cargo check`（F2+F3 交叉接口）；
- 全部合流后冻结候选，跑验收矩阵（§10）；最后合入 `main` 一次。
