# P5-F416 Suspension schema and identity current checkpoint

状态：Ready（N0）。

## 直接父节点

- `P5-D93-suspension-current-base-reconciliation-audit-result.md`

D93 已在 post-F415 current tree 关闭 G0，冻结 current owner、generation 与 N0–N5 DAG。本节点只实现
所有后继共同依赖的 schema/identity 原子检查点；不迁移 compiler、deployment、runtime、Router 或
tooling consumer。

## 精确起点与DAG

- implementation start：
  `0517ec481a19b6cac941ca78ea52e276096f96b3`。
- 必须证明该 commit 是 HEAD ancestor，且 accepted F415
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d` 仍为 ancestor。
- 完成后解除 F417 compiler、F418 deployment、F419 runtime 三个并行节点。
- 当前成熟度：共享实现检查点；允许未迁移下游暂时不能编译，不能宣称稳定候选。

## 独占写入范围

```text
artifact-model/**
artifact-identity/**
本任务result
```

禁止修改 compiler、deployment、runtime、Router、scripts、test-runner、cross-system fixture、
ecosystem source或设计。

## 必须删除的 requirement/protocol facts

1. 删除 `InterfaceMethodSignature.may_suspend`。interface requirement 只保留 receiver、参数、返回值与
   flags shape；不增加 default、alias 或其它 effect 位。
2. 删除 `BoundaryCallbackOperation.may_suspend`。
3. 删除 `BoundaryOperationContract.may_suspend`、`.cancellation`，
   `BoundaryCancellationContract` enum及其 re-export。
4. strict serde 必须拒绝旧 `maySuspend` / `cancellation` 字段。
5. public-instance validation 仍必须：
   - 比较 normalized interface/concrete type shape；
   - 比较 public concrete signature 与 exact method link 的 `may_suspend`；
   - 不再把 interface requirement 的 effect 加入比较。

## 必须保留的 concrete facts

不得删除、default或弱化：

- `ExecutableSignatureIr.may_suspend` / `ExecutableIr.may_suspend`；
- `CallableMayEffects.may_suspend`；
- `CallableSemanticFacts.effects`；
- `BoundaryImplementationRequirements.complete_may_effects`；
- `PackageCallableSignature.may_suspend`；
- `CanonicalPublicCallableSignature.may_suspend`；
- actor/native/builtin concrete summaries。

`PackageRequirement.collection_name_mapping` 与
`PackageBinding.collection_name_mapping`、其 validation及 canonical preimage必须完整保留。

## 原子 generation 切换

| domain | current | terminal |
| --- | --- | --- |
| PackageUnit schema | v1 | v2 |
| implementation-links prefix | v1 | v2 |
| legacy Package build marker/prefix | v2 | v3 |
| PackageArtifact schema | v8 | v9 |
| canonical Local ABI marker/prefix | marker v4 / prefix v6 | marker v5 / prefix v7 |
| canonical build marker/prefix | marker v7 / prefix v9 | marker v8 / prefix v10 |
| PackageSchemaType marker/prefix | v1 | v2 |
| ServiceContractDefinition | v3 | v4 |
| ServiceContract schema | v4 | v5 |
| ServiceProtocol marker/prefix | v4 | v5 |

以下保持：

- FileIR v8；
- Publication ABI v1；
- legacy Package Local ABI v2；
- PackageSchemaIndex v1；
- ContractOperation v1；
- ServiceDeploymentInput v3；
- ServiceDeployment schema/identity v2；
- RuntimeAssembly schema/identity v2；
- pointer/path framing。

所有提升项必须切 strict single-current generation：旧 top-level、marker、prefix和旧字段拒绝；不能
dual-read、dual-write、default、fallback或复用已占用 generation。

## Identity 与负例矩阵

测试至少证明：

1. interface同 shape 的 concrete `may_suspend=false/true` 都能通过 requirement shape validation；
2. public concrete signature与implementation link summary不等仍拒绝；
3. concrete public summary mutation：
   - `PackageCallableId` 稳定；
   - canonical Local ABI/build改变；
4. callback implementor summary变化不改变同 shape PackageSchemaType；
5. provider concrete summary变化不改变 ServiceContract canonical body、protocol identity或
   ContractOperationId；
6. request/response/stream/callback shape变化仍改变 schema/protocol；
7. legacy三类字段、old generation与prefix全部fail closed；
8. collection mapping：
   - missing/empty与map insertion order保持canonical；
   - non-empty mapping/target mutation继续改变package build而不改变Local ABI；
   - invalid/colliding mapping继续拒绝。

deployment/assembly model结构保持v2；若 identity tests需要构造两个 exact refs，只能验证值随 nested
ref变化，不得新增 summary wire。

## 验证与交付

先 `-- --list` 记录实际数量，再运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked --manifest-path artifact-model/Cargo.toml --lib
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked --manifest-path artifact-identity/Cargo.toml --lib
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked --manifest-path artifact-identity/Cargo.toml --test identity_cli
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo check --locked -p skiff-artifact-model -p skiff-artifact-identity
cargo fmt --all -- --check
git diff --check
```

若共享 target 被其它 worktree provenance 污染，可使用本 worktree 下有界隔离 target 重跑并记录；不得
清理共享 cache。不得运行 workspace/full isolated/stable/live，不得派子 Agent。

写 `P5-F416-suspension-schema-identity-current-checkpoint-result.md`，记录 exact commit/tree、
完整 generation/marker/prefix 表、old-wire rejection、identity mutation矩阵、mapping保留与测试计数。
提交并保持 clean；不 merge/rebase/push。
