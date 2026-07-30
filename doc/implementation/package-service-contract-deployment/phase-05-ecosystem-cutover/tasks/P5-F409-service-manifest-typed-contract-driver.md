# P5-F409 Service manifest typed contract and driver

状态：Ready。

## 直接父节点

- `P5-F408-package-public-graph-service-manifest-parser-result.md`
- `P5-F410-deployment-exact-callable-binding-result.md`

F408已经让Package producer与service selection完全分离，并把
`service.yml.serviceCalls`校验为canonical sorted public paths。F410已经让deployment只消费exact
`PackageCallableId`。本节点拥有两者之间唯一剩余的数据流：

```text
serviceCalls public roots
  -> compiler/contract typed selection
  -> ServiceContract + exact operation/callable map
  -> compiler/driver generated deployment input
```

## DAG位置与候选

- DAG节点：F408后的typed selection、contract与driver producer。
- start commit：`be2a1a8a7b893a44c98918114529f32d18ea963c`。
- 完成后解除：F412/F413 ecosystem migration与F414 integrated acceptance。
- 风险：高；ServiceContract operation set、protocol identity与generated deployment identity。

## 独占写入范围

```text
compiler/contract/**
compiler/driver/**
compiler/Cargo.toml
compiler/tests/service_call_roots.rs（必须改名为manifest-selection语义）
compiler/tests/generated_service_deployment.rs
compiler/tests/http_gateway_projection.rs
compiler/tests/service_conformance.rs
compiler/tests/artifact_model_conformance.rs
compiler/tests/builtin_canonical_spelling.rs
compiler/tests/common/package_project.rs（仅在真实fixture helper需要时）
本任务result
```

禁止修改artifact model/identity、F408 producer/parser owners、deployment、runtime、router、
test-runner、ecosystem source或权威设计。

## 必须实现

### 1. 唯一typed selection owner

1. `compiler/contract`是`serviceCalls`字符串解析为typed callable的唯一owner。建议使用内部
   `ServiceCallSelection`，至少保存canonical sorted roots与
   `stable operation path -> exact PackageCallableId`。
2. `project_service_api`显式接收selection paths；不得读取PackageArtifact中的selection、不得从
   deployment或runtime反推。
3. selection在contract边界再次拒绝duplicate后canonical sort，保证直接API调用者也不能绕过F408
   parser；数组顺序不进入任何identity。
4. 每个root只按Package Local ABI完整public graph解析：
   - `PackageLocalAbiSymbol::Callable`且其ID不属于任何public-instance method：生成一个operation；
   - `PackageLocalAbiSymbol::PublicInstance`：展开其`methods`中全部listed-interface methods，
     stable operation path为`root.method`；
   - 精确等于任一public-instance method path的selection必须拒绝，即使同一路径也存在Callable
     symbol；
   - Type、Constant、unknown path必须结构化拒绝；
   - 两个不同roots最终映射同一exact callable必须拒绝。
5. 每个selected exact callable必须存在boundary projection：
   - `Available`进入operation与schema closure；
   - 所有selected unavailable method/function及其reasons聚合报告；
   - missing projection fail closed。
6. Package完整API visibility继续展示所有public callables；只有selected operation带
   `service_operation_id`。未选中的Available callable仍是Package public API，不进入ServiceContract。
7. missing/empty selection生成稳定zero-operation ServiceContract；不得要求至少一个operation。
8. 保留ServiceContract v4、protocol identity v4与已有Package schema reachable closure算法。

### 2. Compiler driver单一流

1. 删除`compile_package_with_service_call_roots`、`service_call_paths`、ordinary/service marker gate及
   所有旧builder测试。`compile_package`始终只编译Package。
2. `compile_service_package`先调用同一个`compile_package`，再把
   `service_root.service.service_calls`交给`project_service_api`。不得恢复service-only source owner。
3. generated deployment的operation input直接clone
   `ServiceApiProjection.available`中的exact ID：

   ```text
   ServiceDeploymentOperationInput {
     contract_operation_id,
     package_callable_id
   }
   ```

   删除Local ABI public-path reverse scan、`package_public_path`及任何fallback。
4. `generated_revision`对`serviceCalls`使用canonical sorted projection。`[a,b]`与`[b,a]`必须得到
   相同revision/deployment identity；missing与`[]`相同。不得靠原始YAML数组顺序。
5. selection集合变化且operation集合变化时，ServiceProtocolIdentity、operation bindings、
   deployment revision与deployment identity相应变化；PackageArtifact/build与Local ABI必须保持相同。
6. 同一source callable可同时被`serviceCalls`选择并被HTTP handler引用；service operation与gateway
   identity保持两个独立domain。

### 3. Fixture与generation同步

1. 把owned测试中的旧`api.yml {source, serviceCall}`全部改为scalar selector，并在对应
   `service.yml`写`serviceCalls`。
2. 把`service_call_roots`测试与Cargo test target改名为
   `service_calls_manifest_selection`或同等明确名称；不能保留旧模型测试名。
3. owned PackageArtifact literals删除`service_call_roots`，schema/build固定切v8/v9；canonical std
   build pin使用当前v9事实，不做prefix-only伪替换。
4. owned `ServiceManifestAuthoring` literals补`service_calls`。
5. `generated_service_deployment`、HTTP dual-surface与stream conformance fixture全部走真实
   Package+Service root和新selection。

## 必须覆盖的正反例

- 选择ordinary function；
- 选择public instance时完整展开全部listed methods；
- 直接选择`worker.run`拒绝；
- unknown、Type、Constant、instance-method alias、duplicate exact callable拒绝；
- missing boundary拒绝，多个Unavailable一次聚合；
- missing/empty selection的zero-operation稳定性；
- `[a,b]`与`[b,a]`的protocol、revision、deployment identity完全相同；
- `{a}`与`{a,b}`的PackageArtifact/build/Local ABI完全相同，但contract/deployment按operation集合变化；
- generated operation input/output保留同一exact callable ID且没有public-path字段；
- 同一source同时作为service call和HTTP handler时identity domain分离。

## 验证

先用`-- --list`记录实际选择，再运行至少：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-contract projection
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler pipeline
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler --test service_calls_manifest_selection
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler --test generated_service_deployment
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler --test http_gateway_projection
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler --test service_conformance
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler --test artifact_model_conformance
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler --test builtin_canonical_spelling
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo check --locked -p skiff-compiler
cargo fmt --all -- --check
git diff --check
```

可按真实selector名调整，但不得使用零测试作为通过。反向搜索必须证明compiler production/test不再有
`PackageServiceCallRoot`、`service_call_roots`、selection意义的`service_call`、旧API marker、
`package_public_path` deployment writer、v7 artifact或v8 build pin；保留的FileIR
`service_call_refs`/call-site lowering必须分类列出。

不得运行完整workspace/isolated/stable/live或生态publish，不得派子Agent。若实现要求改动禁止owner，
或typed selection无法仅凭Package public graph完成，停止并如实上报，不得扩大范围。

## 交付

写`P5-F409-service-manifest-typed-contract-driver-result.md`，记录exact start/end commit/tree、typed
selection算法、identity正反例、旧模型反向搜索、测试实际计数与仍待integration重跑的gate。提交并保持
clean，不merge/rebase/push。
