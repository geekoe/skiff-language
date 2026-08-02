# Actor 实例状态与并发块统一求值设计

> Status: 设计稿，待独立 review（可行性 / 正确性）。
> 相关文档：[actor-model.md](actor-model.md)、[runtime.md](../reference/runtime.md)（§6 Concurrent lane model）。
> 本文只描述目标态设计，不承诺实现完整性；实现计划另行给出。

> **本文档已被 [actor-shared-heap-design.md](actor-shared-heap-design.md) 取代。**
> 设计方向从“canonical 字段表 + 每方法 heap”改为“实例共享 arena + 段级借用（reacquire）”。
> 本文保留作为决策演进与 review 历史参考，不再作为目标态设计。

## 1. 背景与问题

当前 actor 执行实现（`runtime/eval`）中：

- 每个方法执行持有一个独立 `RequestHeap`（arena），字段状态存放在实例 store 的 heap 中；
- 每次真实挂起时 `snapshot_persistent_fields` 把字段根深拷贝到 compact heap 并 commit（`suspend`），恢复时再从 store 深拷贝回续体 heap（`resume`）；
- 每次字段写入走 boundary wire 往返（`to_wire_json` / `from_wire_json`）；
- actor 方法内 `concurrent` block 通过 E3 bridge 为每个 lane 建立独立执行 lease（per-lane slot / gate / 嵌套状态机），因为 lane 是独立求值器实例（独立 env + heap 克隆）。

由此产生的成本与复杂度：

- 挂起成本 O(字段图) 无条件发生，即使该段没有写任何字段；
- 恢复成本 O(字段图) 无条件发生，即使挂起期间没有其他方法提交字段；
- E3 bridge 约 900 行；
- `concurrent` lane 禁止写外层 mutable root（含 actor 字段）是“每 lane 克隆 heap”模型的产物，不是语义必需。

观察：同实例方法在段级串行执行，字段换手只需要“可见性”，不需要“内存共享”。JS 用共享 GC 堆 + 事件循环达到零复制；Skiff 的 arena / `&mut` 借用模型使共享堆需要重写求值器（约 205 处 `&mut RequestHeap`），因此不选。本文用 **canonical 字段表 + 脏跟踪 + 惰性解码** 替代整图快照，并把 `concurrent` block 统一为单求值实例。

## 2. 目标与非目标

目标：

1. actor 方法真实挂起 / 恢复时零整图复制：只发布、只重取实际变更的字段；
2. 字段写入不再每次 wire 往返（保留一次类型校验）；
3. `concurrent` block 编译为单个执行单元（一个 heap / env + 挂起操作表），删除 per-lane 克隆、const 导出 deep-clone 与 E3 bridge；
4. 保留现有段原子、方法交错、发布-消费可见性语义（与 actor-model.md 一致）；
5. 普通 request 语义与求值器借用模型完全不变。

非目标（v1 不做）：

- 不引入 promise / async-await 语法；
- 不重构 `EvalContext` 的 `&mut RequestHeap` 所有权模型；
- 不做共享 arena + reacquire（远期选项，见 §7）；
- 不改变跨 runtime 调用边界的编解码（actor 调用仍走 router，与 service call 一致）；
- 不放开 `concurrent` lane 写共享字段（v1 仍禁止；技术障碍已消除，属纯语义选择，见 §4.3）。

## 3. Canonical 字段表

### 3.1 存储形态

实例 store 持有：

- `fields: 字段名 -> FieldVersion { generation: u64, value: CanonicalValue }`
  - `CanonicalValue` 是该字段类型的规范编码（复用现有 boundary codec 的 wire 形态），不持有任何 heap handle；
- 实例 store 不再作为字段的权威 heap 宿主；每个方法执行独立持有自己的 heap。

### 3.2 方法生命周期

- 进场（方法开始或恢复）：不整图解码；`read_field` 首次访问某字段时，按该字段 generation 解码到方法 heap，并缓存段内副本。
- 写字段：一次类型校验 → 更新方法 heap 中的段内副本 → 标记该字段 dirty（记录字段名）。
- 真实挂起或方法结束：`publish_dirty()` —— 对每个 dirty 字段编码一次、写回 store、bump generation、清 dirty。
- 恢复：对将要读的字段比较 store generation 与段内缓存 generation；不同则丢弃旧副本并重新解码。
- 只读挂起段：无 dirty，发布为空操作；若挂起期间实例级 generation 未变化，恢复时无需任何重取。

### 3.3 发布与消费者

- “发布” = 方法私有修改写回 store 字段表（编码 + 版本推进）。
- 消费者 = 同一实例的其他方法执行（含挂起后恢复的自身）。字段不进 router、不进 registry（registry 只保存创建输入）；实例逐出即丢弃。
- 可见性点 = 段边界（真实挂起 / 结束），与当前语义一致；同步段内其他方法不可见（本来也不可运行）。

### 3.4 与现有实现的对应

- `snapshot_persistent_fields` / resume 导入 / `write_field` wire 往返 → 删除，或退化为脏字段编码 / 解码；
- `ActorInstanceExecutionLease` 的 fields + heap 克隆 → 删除（store 不再为方法克隆字段；方法 heap 来自执行上下文）；
- `commit_execution` → `publish_dirty` + generation bump；
- `ActorExecutionFrame::suspend` / `resume` → 只负责发布与恢复时的 generation 检查。

### 3.5 语义

- 副本隔离：方法挂起前持有的字段对象副本，在挂起期间不随其他方法修改而更新；恢复后按 generation 重取（保持 actor-model.md 的“恢复后必须假设字段已变化、重新读字段”纪律）。
- 编译器可继续诊断“挂起前读取字段、恢复后继续依赖旧值”的明显模式（已有方向，不新增承诺）。

## 4. Concurrent block 单执行单元

### 4.1 现状问题

lane 是独立求值器实例（独立 env + heap 克隆）：

- per-lane heap 克隆（O(heap)）与 const 导出 deep-clone；
- 禁止 lane 写外层 mutable root（含 actor 字段）是克隆模型的产物；
- actor 方法内 `concurrent` 需要 E3 bridge（per-lane lease / gate / 嵌套状态机）。

### 4.2 目标形态

`concurrent` block 编译为单个求值实例：

- 一个 heap、一个 env（含块级作用域规则）；
- 每个直属语句（或 `serial` 组、tail 表达式）编译为一个可挂起段；
- 运行时维护挂起操作表 `{ segment -> pending future }`；wake 后按“哪个操作完成”恢复对应段，跑至下一真实挂起或段完成；
- 依赖（const 前向可见）由编译器在段序 / 作用域层面表达，不再需要 lane DAG 的运行时机制。

### 4.3 对 actor 的影响

- `concurrent` block 是方法的一部分：方法级发布时机以“块整体挂起”（所有段 pending）为段边界；
- 不再需要 per-lane lease、instance scheduler 竞争、gate / 嵌套状态机（E3 bridge 删除）；
- lane 写字段的技术障碍消失；v1 仍由编译期禁止（保持确定性），放开与否是纯语义决策。

### 4.4 错误与取消

- 段错误：源码序靠前获胜（保持现有规则）；未启动段不启动；运行中段结构化停止；
- 已发布字段写不回滚（与“已提交外部副作用不回滚”一致）；未发布写随方法 heap 丢弃；
- 跨挂起读-改-写（lost update）风险保留：业务在挂起后重新读字段；编译器可诊断明显模式。

### 4.5 作用域

- lane 与块外层共享 env；v1 规则：lane 可读外层 const / actor 字段，不可写外层局部变量；lane 内可声明 const，按源码序对后续段可见（forward reference 仍禁止）。

## 5. 事务与 rollback

- 方法独占 heap：现有 truncate 型 rollback（`nodes.truncate(checkpoint.len)`）在方法内安全——没有其他方法在同一 heap 分配。
- actor 字段回滚：保留 `with_transaction_live_fields` 机制，但需要限制事务与交错写的相互作用。
- v1 决策（待 review 确认）：actor 方法内 db transaction 不允许真实挂起（编译期或运行期拒绝），或事务期间持有实例、不发布、其他方法不得进入；避免“事务挂起 → 其他方法写字段 → 事务回滚覆盖他人写入”。
- 若未来需要事务跨挂起 + 交错写：引入 staging（事务在独立 heap 执行，commit 时把变更字段编码回 store；abort 丢弃），成本为每事务一次字段编码，列为后续设计。

## 6. 实施影响（触点，非实现计划）

- `runtime/model`：`FieldVersion` / canonical 字段存储类型。
- `runtime/eval/actor_instance.rs`：store 形态、acquire / commit、lease 简化。
- `runtime/eval/actor_executor.rs`：删除 snapshot / import；`write_field` 简化；suspend / resume 改发布与版本检查。
- `runtime/eval/actor_executor/actor_concurrent_continuation*.rs`：删除（E3）。
- `runtime/eval/env/concurrent_scheduler*`：替换为单状态机调度。
- compiler：`concurrent` lowering 改段 / 操作表；`actor_method_validation` 规则调整（v1 保持 lane 禁写字段）。
- `program_db` rollback：保持 truncate；增加事务挂起限制。
- 测试：删除 / 重写快照语义测试；新增 generation / 脏跟踪 / 并发块 / 事务限制测试。

## 7. 与共享 heap 的取舍（远期选项）

共享 arena + reacquire（JS 式）的触发条件：

- profile 证明大字段每段变更的编码成本成为热点；
- 需要别名 live 语义；
- 未来允许 `concurrent` lane 写共享字段且需要内存级共享。

当前不做共享 heap 的原因：需要重写 EvalContext 借用模型（205 处 `&mut RequestHeap`）、事务 truncate 失效（需 staging / undo）、内存回收复杂（quiescence 压缩）、别名语义变更（文档与测试面大）。

保留 hybrid 可能：对特定大字段单独共享（store 持有该字段共享副本），其余字段走 canonical，属于后续设计。

## 8. 验收矩阵（行为不变量）

1. 同实例方法交错、段原子、挂起 / 恢复可见性与 actor-model.md 一致；
2. 只读挂起段零字段复制；脏字段发布 / 重取正确（generation 竞态）；
3. `concurrent` block：普通 request 语义不变；actor 方法内 block 与外围方法交错正确；
4. 事务：rollback 不覆盖其他方法已提交字段（v1 通过禁止事务挂起保证）；
5. 升级 / 逐出 / 断连：仍按创建输入重建，字段不跨任期（语义不变）；
6. 跨包 actor 调用（actor-model.md 消费视图 1–4）不回归。

## 9. 开放问题（供 review 重点检查）

- Q1. 发布时机：方法挂起 = 块整体挂起；块内单段挂起但其他段可跑时，其他 actor 方法是否可进入？（建议：不可，方法仍 active）
- Q2. 事务挂起限制（§5）是否可接受；staging 是否值得 v1 做。
- Q3. 字段 generation 粒度：per-field vs per-instance；大字段编码成本是否需字段级混合。
- Q4. `concurrent` 单状态机下，块内控制流限制（if / match / loop）v1 是否保持不变；动态 op 表（循环内 `concurrent`）何时放开。
- Q5. env 共享后的可见性规则（§4.5）是否足够。
- Q6. canonical 编码对字段类型的覆盖：复用 boundary codec 是否足够；stream / request-scoped handle 字段继续禁止。
