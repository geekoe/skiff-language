# P5-F418 Suspension deployment admission

状态：Ready（N2）。

## 直接父节点

- `P5-F416-suspension-schema-identity-current-checkpoint-result.md`

需要核对 deployment current owner或跨节点负矩阵时，再沿父节点引用读取
`P5-D93-suspension-current-base-reconciliation-audit-result.md`。

## 精确起点与任务边界

- integrated N0 checkpoint：
  `c597e3c0e5ecb9d1711b1a25a2660ea9cc972a60`；
- N0 implementation：
  `57d0a5551aaa62e5a71655050478c1447f94324d`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时必须证明三个 commit 都是 HEAD ancestor。本节点可与 F417 / F419并行；独立分支上的 fixture仍可能等待
compiler fresh artifact，最终 combined proof由主 Agent在合流后执行。

独占 production 写入范围：

```text
deployment/**
本任务 result
```

核心 owner：

```text
deployment/src/projection/eligibility.rs
deployment/src/projection/operations.rs
deployment/src/projection/tests.rs
deployment/src/projection/tests/eligibility.rs
deployment/src/projection/tests/operation_bindings.rs
deployment/src/storage/tests.rs
deployment/src/assembly/tests/fixtures.rs
```

禁止修改 artifact-model、artifact-identity、compiler、runtime、router、scripts、test-runner、
cross-system fixture、ecosystem source或设计；不得派子 Agent。

## 必须实现的终态

deployment admission顺序保持：

1. exact operation shape与 callable binding；
2. exact `CallableSemanticFacts` 与 `BoundaryImplementationRequirements`；
3. detached / escape / mutation / callback / stream eligibility；
4. `effects == complete_may_effects` 与 provenance equality。

只删除：

- `effects.may_suspend != contract.may_suspend` 一类 provider-summary / code-free-contract比较；
- `BoundaryCancellationContract::Unsupported` 等已删除 cancellation field的feature branch；
- 对应旧 fixture与断言。

不得删除或放宽：

- unknown effects fail closed；
- complete effect mismatch；
- provenance mismatch；
- operation shape / callable binding mismatch；
- unsupported stream等独立 capability限制。

同一 code-free `ServiceContractRef` 必须能绑定 concrete `may_suspend=false` 或 `true` 的 provider。两个
provider的 exact Package build ref不同，因此 deployment和assembly identity value可以不同；这不允许给
ServiceContract重新增加provider bit。

## F415 mapping preservation

以下 production validator不可删除或放宽：

```text
deployment/src/projection/package_closure.rs
deployment/src/assembly/resolver.rs
```

`PackageRequirement.collection_name_mapping`、deployment
`PackageBinding.collection_name_mapping` 与 assembly package link必须逐跳 exact相同。若 fixture需要适配，
同一 dependency edge必须携带同一显式 map；不得用 empty fallback、model default或删除 drift/collision
negative。

## Generation与identity边界

- 输入只接受 N0 terminal PackageArtifact v9、Local ABI v7、build v10、ServiceContract /
  ServiceProtocol v5。
- ServiceDeploymentInput仍为 v3；ServiceDeployment schema / identity仍为 v2。
- RuntimeAssembly schema / identity仍为 v2。
- 不增加 summary wire，不提升 deployment / assembly generation，不做兼容读取。

测试至少证明：

- 同一 contract + concrete false/true provider均通过；
- 两个 exact provider build refs令 deployment / assembly value不同；
- contract protocol / operation identity不依赖provider suspension summary；
- unknown effects、complete effect、provenance、shape与mapping mismatch仍拒绝；
- non-empty mapping从 requirement到binding到assembly link保持 exact。

## 验证与交付

先用相同 selector加 `-- --list` 记录实际数量，再运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-deployment projection
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-deployment storage
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-deployment assembly
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo check --locked -p skiff-deployment
cargo fmt --all -- --check
git diff --check
```

D93 accepted listing基线依次为 `19 / 13 / 20`；以当前实际 listing为准并解释合理变化。不要运行
workspace/full isolated/stable/live。

写 `P5-F418-suspension-deployment-admission-result.md`，记录 exact commit/tree、admission保留/删除
矩阵、同一contract的两个provider证据、deployment/assembly identity、mapping逐跳证据、实际测试计数和
combined-tree待验证项。提交并保持 clean；不 merge/rebase/push。

若一次有界探查后发现必须越过授权 production root、公共契约仍不明确或任务实际拆成多个新 owner，停止并返回
`TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`，不要自行扩大范围。
