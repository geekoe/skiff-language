# P5-F49：Recoverable Owner Lazy Validation

DAG节点F49，依赖D49 COMPLETE。权威设计为
`doc/architecture/package-service-contract-deployment.md` §10/§12与
`doc/architecture/recoverable-value.md`的LocalConcrete owner/key条款。

从integration checkpoint创建独立worktree。唯一写入范围：

- `runtime/eval/src/recoverable_behavior.rs`及其同模块聚焦测试。

删除`EvalRecoverableBehaviorHooks::new_for_execution`对assembly-wide packageId唯一性的eager检查及死代码。
保留真正使用package-owned LocalConcrete时的按需0/多candidate fail-closed、restore-key conflict检查。
不得修改fixture、resolver、execution image、model/wire/codec，禁止把build/version/slot/artifact identity放入
durable key，禁止first-win、按build挑选或compat/dual path。

至少新增并运行：

- duplicate packageId/different build + plain-data canonical hook construction成功；
- 同assembly实际需要package-owned LocalConcrete时仍以packageId ambiguous fail closed；
- 既有recoverable与spawn聚焦回归；
- changed-file rustfmt、`git diff --check`及禁止方案反搜。

完成后提交单一commit并返回自验收矩阵。禁止完整I02/R05/instance/stable/full gate；本提交只形成实现检查点，
必须经合流后的I49 cheap combined才解除下一次I02。
