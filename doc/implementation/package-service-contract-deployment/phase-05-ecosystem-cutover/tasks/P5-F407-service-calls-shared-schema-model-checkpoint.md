# P5-F407 Service-calls shared schema/model checkpoint

状态：Ready。

## 直接父节点

- `P5-F403-service-calls-manifest-implementation-audit-result.md`

父节点以F402权威设计为准，已列出全部model/identity owner、代际与下游DAG。本节点只实现所有consumer
共同依赖的schema/model checkpoint；不迁移compiler、deployment、runtime或ecosystem consumer。

## DAG位置与候选

- DAG节点：serviceCalls cutover共享N0。
- start commit：`9836e76f`的完整identity由启动时记录。
- 完成后解除：F408 producer/parser、F410 deployment consumer、F411 runtime/router/test fixtures并行。
- 当前成熟度：实现检查点；允许未迁移下游crate暂时无法编译，不能宣称稳定候选。
- 风险：高；strict artifact wire、build identity与deployment input。

## 独占写入范围

只允许：

```text
artifact-model/**
artifact-identity/**
scripts/check-artifact-identity-single-source.mjs
scripts/tests/check-artifact-identity-single-source.test.mjs
本任务result
```

不得修改compiler、deployment、runtime、router、test-runner、ecosystem source或canonical design。

## 必须实现

### PackageArtifact

- 删除`PackageServiceCallRoot`类型、re-export与`PackageArtifact.service_call_roots`字段。
- PackageArtifact schema `skiff-package-artifact-v7`升为v8。
- strict serde不得dual-read、default或接受旧`serviceCallRoots`。
- Package build identity projection删除selection：
  - preimage marker v6→v7；
  - identity prefix `skiff-package-build-v8:sha256:`→v9。
- PackageLocalAbi marker/prefix与preimage保持不变。
- roots validator与只验证roots的forgery/identity测试删除；public instance surface、callable links、
  boundary projections与`service_call_refs` validation必须保留。

### Service authoring

在`ServiceManifestAuthoring`增加：

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub service_calls: Vec<String>
```

字段wire name为`serviceCalls`。missing与`[]`等价为zero selection；此层不解析public path。

### Deployment input

- `ServiceDeploymentOperationInput.package_public_path`改为exact
  `package_callable_id: PackageCallableId`。
- input schema `skiff-service-deployment-input-v2`升v3。
- strict wire拒绝旧`packagePublicPath`，不得保留fallback。
- 最终ServiceDeployment model/generation与identity不变。
- single-source checker同步required fields，继续拒绝第二owner。

## 身份与负例

测试必须证明：

- v8 artifact无`serviceCallRoots`，人工旧字段被拒绝；
- build v9 preimage不含selection，Local ABI receipt保持原generation；
- public-instance完整method surface仍在Local ABI/links/boundary中；
- `service_call_refs`不被误删；
- service manifest missing/empty `serviceCalls`按规定解析；
- deployment input v3只接受exact callable ID，旧path/unknown field拒绝；
- forged/missing artifact identities继续fail closed。

这是schema切代，v7→v8时同source build identity变化属于预期；“只改serviceCalls而PackageArtifact
bit-identical”由新模型中PackageArtifact完全不读取manifest selection保证。

## 验证

先用`-- --list`记录实际选择数，再运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-artifact-model package_artifact
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-artifact-model ecosystem_authoring
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-artifact-model deployment
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-artifact-identity package_artifact
node --test scripts/tests/check-artifact-identity-single-source.test.mjs
cargo fmt --all -- --check
git diff --check
```

若selector名字与真实test module不同，可用`-- --list`确定等价最小selector并在result记录；不得用零测试
冒充通过。不得运行workspace/full isolated/stable/live，不得派子Agent。

## 交付

写`P5-F407-service-calls-shared-schema-model-checkpoint-result.md`，记录start/end commit/tree、
generation/prefix、wire正负例、test计数、反向搜索与自验收矩阵。提交全部改动并保持clean；不
merge/rebase/push。
