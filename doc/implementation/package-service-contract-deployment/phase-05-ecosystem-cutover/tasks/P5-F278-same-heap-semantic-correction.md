# P5-F278 Same-heap semantic correction

状态：Ready。

## 直接父节点与权威链

- 直接父结果：
  `P5-F277-same-heap-semantic-correction-result.md`
- 该结果继续引用 F276/F275；
- 唯一架构事实源：
  `doc/architecture/package-service-contract-deployment.md` 第 7、8 节。

启动时只读本任务；需要依据时沿上述父链向上读取。

## DAG 位置与共享状态

- 输入：`codex/package-service-phase-05` 上包含 F277 权威修正的 docs checkpoint。
- 前置：F271/F273 structured provenance 已合流；F277 语义已冻结。
- 当前成熟度：实现检查点，不是稳定候选。
- 完成后解除：AIHub 8/8 fresh republish、F269 Internals test-service 总验收。
- F268 test-service migration 和 error-channel 设计是并行表面，不得修改。

## Production owner 与首次损失点

父结果确认 AIHub 首次误报来自 alias/mutation 与 identity observation 共用同一 aggregate 位。
production setter/consumer 至少包括：

- `artifact-model/src/builtin_receiver_ops.rs`
- `compiler/source/src/callable_effects/provenance.rs`
- `compiler/source/src/callable_effects/transfer/{call,expression,statement}.rs`
- `compiler/projection/src/package_artifact/boundary/eligibility.rs`

测试 owner 至少包括：

- `compiler/source/src/callable_effects/tests.rs`
- `test-runner/tests/package_service_contract_deployment.rs`
- 上述 production module 的就地单元测试。

必须反向搜索所有 `requires_same_heap_identity` setter；不能只修改 `Map.get`。

## 写入范围与完成标准

1. 只有 caller-reachable heap value 的 `==` / `!=` 或明确 identity intrinsic 可以产生
   `requiresSameHeapIdentity`。
2. Map/JsonObject get 保留精确 caller projection 和 `returnsCallerAlias`，但 identity 位为 false。
3. Array/Map/JsonObject mutation 和 field store 保留 write/parameter-store facts，但 identity 位为
   false。
4. interface boxing/callback、ordinary throw/rethrow 和 unresolved/unknown target 不单独产生 identity
   位；它们继续由各自 escape/throw/unknown fact 拒绝。
5. `all_effects`/fail-closed 路径必须仍可靠拒绝 unknown，但不得把 unknown 报成已发生的 identity
   observation。
6. boundary eligibility 对真实 identity observation 无条件给出
   `RequiresSameHeapIdentity`；不得用 detached parameter、DB materialization 或 fresh consumer 在
   observation 已发生后放行。
7. 直接返回 get 仍因 `ReturnsCallerAlias` unavailable；caller mutation 仍因
   `WritesCallerReachable` unavailable；fresh local get/mutation 可保持无 caller-visible effect。
8. 增加/更新正负测试，至少覆盖：
   - caller reference equality / inequality 仍为 true 并被 boundary 拒绝；
   - fresh reference equality 不上浮；
   - get alias、caller mutation、interface boxing、unknown target 各自不伪造 identity；
   - 跨 local helper/SCC 的 write 与真实 identity propagation；
   - test-runner mutating helper reason 不再包含 identity。

## 非目标与禁止范围

- 不扩展 PackageArtifact、provenance 或 service contract wire。
- 不改变 runtime `==` / `!=` 行为。
- 不修改 AIHub、Agine、skiff-packages 或 internals。
- 不修改 ErrorPayload、throw/catch 或 error envelope。
- 不放宽 alias、write、escape、callback、native 或 unknown boundary gate。
- 不运行 stable instance、完整 ecosystem gate 或 packages 全量测试。

## 验证 owner

本任务唯一拥有以下聚焦验证：

```bash
cargo test -p skiff-artifact-model builtin_receiver_ops
cargo test -p skiff-compiler-source callable_effects
cargo test -p skiff-test-runner --test package_service_contract_deployment
cargo fmt --check
git diff --check
```

若测试选择器实际为零测试，必须改用真实可列出的等价聚焦命令并报告。

## Worktree、提交与交付

- worktree：`/Users/geek/workspace/skiff-p5-f278-same-heap`
- branch：`codex/p5-f278-same-heap`
- 不回滚或覆盖其它 worktree 的改动，不 push。
- 从启动到第一次 production 代码修改不超过 5 分钟；若合同不可执行，立即返回
  `TASK_NOT_EXECUTABLE`、精确缺口和最小前置。
- 完成后提交一次或少量有序 commits，并返回提交、变更摘要、未决问题及自验收矩阵。

