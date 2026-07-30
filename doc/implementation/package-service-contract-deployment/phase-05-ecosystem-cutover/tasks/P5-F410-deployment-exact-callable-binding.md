# P5-F410 Deployment exact callable binding

状态：Ready。

## 直接父节点

- `P5-F407-service-calls-shared-schema-model-checkpoint-result.md`

F407已把`ServiceDeploymentOperationInput`切为v3及exact
`PackageCallableId`。本节点只迁移deployment consumer；compiler driver producer由F409拥有。

## DAG位置与候选

- DAG节点：F407后的deployment consumer。
- start commit：`288a105fc87399c5e93228ee9f2ba2e58c4cd2b6`。
- 可与F408/F411并行；集成后等待F409 producer。
- 风险：高；deployment admission与operation binding。

## 独占写入范围

```text
deployment/**
本任务result
```

禁止修改artifact model/identity、compiler、runtime、router、test-runner、ecosystem source和设计。

## 必须实现

1. `project_operation_bindings`直接消费
   `ServiceDeploymentOperationInput.package_callable_id`，删除`package_public_path`查找及任何path fallback。
2. exact ID必须存在于implementation PackageArtifact，并且是Package Local ABI的public function或
   public-instance method；implementation-only/private callable必须拒绝。
3. 继续逐项验证：
   - contract operation存在且不重复；
   - boundary projection Available且descriptor精确匹配；
   - callable semantic facts、implementation requirements与link target一致；
   - operation set无遗漏/额外；
   - final `DeploymentOperationBinding`保存同一exact ID。
4. forged/missing/non-public ID、wrong contract ID、Unavailable、facts/descriptor/link mismatch全部
   fail closed。
5. 不读取`service.yml`、不解析public path、不重建selection。
6. ServiceDeployment output/generation与identity保持v2。

## 验证

更新deployment全部v3 input literals并新增上述正负例。先列出实际测试数，再运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-deployment projection
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-deployment storage
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo check --locked -p skiff-deployment
cargo fmt --all -- --check
git diff --check
```

若deployment crate中所有fixtures必须机械删除`service_call_roots`或切v8/v9，属于本consumer范围；不得
修改其它crate。不得运行workspace/isolated/stable/live，不得派子Agent。

## 交付

写`P5-F410-deployment-exact-callable-binding-result.md`，记录exact commit/tree、旧path反向搜索、exact-ID
验证矩阵与测试计数。提交并保持clean，不merge/rebase/push。
