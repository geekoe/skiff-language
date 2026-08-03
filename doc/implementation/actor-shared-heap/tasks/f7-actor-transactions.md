# F7：actor 内 db transaction 恢复支持（DB-only 语义）

> 直接父节点：权威设计 `doc/architecture/actor-shared-heap-design.md` §5（v1 修订版）。
> 决策（用户确认）：事务只回滚 DB，不回滚 actor 内存；事务体内禁止写 actor 字段（编译期）；
> 不做字段快照，一致读走 DB。

## 写集

- compiler：删除 execution_semantics 的 actor 事务禁令（`effects.rs` 移除、`owner.rs`/
  `mod.rs` 清理）；`actor_method_validation.rs` 新增“事务体内禁止写 actor 字段”校验
  （直接赋值 + 字段接收器原地修改）；
- runtime/eval：`program_db.rs` 新增 `rollback_after_transaction`——actor 上下文走 DB-only
  分支（不 truncate 共享 arena、不 rebase Env），普通 request 保留原 truncate + rebase；
- 测试：compiler 正向（actor 方法/create/本地 helper 事务可编译）与负向（事务体内字段写拒绝）；
  runtime actor 事务成功与 abort 轨迹测试；
- 文档：设计 §5 修订、interfaces.md 备注。

## 验收

- agent 包（internals/packages/agent）以新编译器 `package build` 通过（7 个 actor 方法不再报错）；
- `skiff-compiler-source` 352、`skiff-compiler-lowering` 74、`skiff-compiler` 全量、
  `skiff-runtime-eval` 442 + program_db 事务 6 全绿。
