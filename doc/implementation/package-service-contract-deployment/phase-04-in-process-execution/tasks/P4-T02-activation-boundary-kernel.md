# P4-T02：Activation / Boundary / Capability Kernel

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2.8–§2.10、§6.2、§7、§11、§12、§14。
- 风险/验收组：高风险owner/lifetime/materialization；与T01/T03合流后由R01验收。
- 当前成熟度：planning document checkpoint；完成后只是activation/boundary kernel checkpoint。
- 有效证据：本任务clean commit及exact doc checkpoint。ActivationContext、callback carrier/table、contract
  materializer、Cargo DAG或本任务测试变化会使证据失效。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：P4-D01 PASS；可与T01并行。
- 解锁：T03。
- branch：`codex/p4-t02-activation-boundary-kernel`。
- worktree：`/Users/geek/workspace/skiff-p4-t02-kernel`。
- 五分钟内真实edit；此前不跑测试。若必须让activation依赖eval/host、复制ServiceContract descriptor或引入
  RemoteBoundary，立即回报`TASK_NOT_EXECUTABLE`。

## 写入范围

- `runtime/model`：最小opaque callback-capability runtime carrier及exhaustive model/heap测试。
- `runtime/boundary`：新增service-linkable contract plan/materializer模块；不要继续扩大`binary.rs`/`recoverable.rs`。
- `runtime/activation`：新增ActivationContext、request generation/lifecycle、binding vector、capability table和kernel error/API。
- 必要Cargo manifest/DAG声明。不得修改linked-program/linker/eval/request/host/router/compiler。

## 完成态

1. `ActivationContext`按assembly identity + generation + runtime replica + deployment唯一；同package build的两个
   activation共享immutable code引用但不共享binding/config/state/resource/callback mutable owner。
2. runtime binding key固定为`(callerPackageBuildId, serviceRequirementSlot)`，value只有本地provider activation、
   exact contract与used operations；缺失/mismatch在invoker前失败。
3. `RequestActivationContext`显式携带receiver/current owner、request generation、cancel和stream lifetime；无TLS。
4. plan-aware materializer直接消费canonical `ContractTypeRef + boundary_schema + BoundaryValuePlan`，在fresh heap中
   detached materialize普通graph，拒绝Unsupported、缺schema、错误carrier/encoding/owner/lifetime。
5. opaque callback carrier只含设计字段，不含method table/native object/address；activation-owned table支持
   register/lookup/expire，区分expired与unavailable，不重建/fallback。
6. materializer为callback/native lane只提供显式hook；普通detached path遇到local interface/native handle fail closed。
7. callback capability不能通过recoverable/DB/spawn/queue已有encoder；不改变普通deep clone与package direct语义。

## 最早探针与唯一验证 ownership

```bash
cargo test -p skiff-runtime-model callback_capability
cargo test -p skiff-runtime-boundary service_linkable
cargo test -p skiff-runtime-activation activation_context
node scripts/check-runtime-crate-dag.mjs
git diff --check
```

探针必须覆盖activation隔离、slot tuple、detached alias隔离、plan负例、wrong generation/owner/lifetime与
recoverable拒绝。只格式化本任务文件，不运行完整gate。

## 回报

提交一个commit，回报owner/lifetime状态机、materialization matrix、public hooks、命令与自验收矩阵。
