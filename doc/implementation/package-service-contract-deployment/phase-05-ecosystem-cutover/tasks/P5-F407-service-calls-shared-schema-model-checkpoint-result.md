# P5-F407 Service-calls shared schema/model checkpoint result

状态：Complete。

## 1. 提交锚点与范围

| 锚点 | commit | tree |
| --- | --- | --- |
| 任务规定 start | `9836e76fe2a3e18b0556431900204ef1a32f6167` | `0e877a0bbefb1b5de3e3ed1b5c9561e94f1b1510` |
| task definition checkpoint | `c6afdd95707b0c6ea3efe7c8d8a51b802b261fa9` | `6c197993258b21d30866ab1ee3cb64719d0aa3ef` |
| implementation end candidate | `3bcc89581021790210f4d39fa5d41214ee2de86d` | `e389e0e9e678fe7ede785ea4cfe99d47586aa2d5` |

实现提交：

```text
3bcc89581021790210f4d39fa5d41214ee2de86d
feat(artifact): detach service call selection
```

production/test 改动只落在任务授权的：

```text
artifact-model/**
artifact-identity/**
scripts/check-artifact-identity-single-source.mjs
scripts/tests/check-artifact-identity-single-source.test.mjs
```

本文是唯一额外 result 文件。没有修改 compiler、deployment、runtime、router、test-runner、
ecosystem source 或 canonical design；没有承接后续 DAG 节点。

## 2. Generation 与 identity

| surface | start | end | 结论 |
| --- | --- | --- | --- |
| PackageArtifact wire | `skiff-package-artifact-v7` | `skiff-package-artifact-v8` | required `serviceCallRoots` 已删除；旧字段作为 unknown field 拒绝 |
| Package build preimage marker | `skiff-package-artifact-build-identity-v6` | `skiff-package-artifact-build-identity-v7` | preimage 不再含 selection |
| Package build identity prefix | `skiff-package-build-v8:sha256` | `skiff-package-build-v9:sha256` | preimage generation 切代 |
| PackageLocalAbi marker | `skiff-package-artifact-local-abi-identity-v4` | 不变 | projection 未改 |
| PackageLocalAbi prefix | `skiff-package-local-abi-v6:sha256` | 不变 | receipt generation 未改 |
| ServiceDeploymentInput wire | `skiff-service-deployment-input-v2` | `skiff-service-deployment-input-v3` | operation input 改为 exact callable ID |
| ServiceDeployment wire / identity | v2 / v2 | 不变 | canonical output 原本已保存 exact callable ID |
| ServiceContract / protocol | v4 / v4 | 不变 | 本节点未改 contract |

同 source 从 v7 迁到 v8 时，build preimage 少一个 member并使用 v9 prefix，因此 build identity
变化是本次 schema migration 的预期结果。Local ABI 的 marker、prefix、projection字段均没有变化。

## 3. Model、wire 与 validation 结果

### 3.1 PackageArtifact

- 删除 `PackageServiceCallRoot` enum、`public_path()`、crate re-export 与
  `PackageArtifact.service_call_roots`。
- `PackageArtifact` 仍是 `deny_unknown_fields`，没有为旧字段增加 `default`、alias、dual-read或
  compatibility adapter。
- v8 正例不输出 `serviceCallRoots`；人工加入完整旧 function/public-instance roots 后反序列化失败。
- `packageBuildId`、`packageLocalAbi.localAbiIdentity` 或 `serviceCallRefs` 缺失仍然失败。
- build identity projection删除 roots clone/sort/member；marker/prefix同步切为v7/v9。
- roots-only validator与roots forgery/identity tests已删除。

### 3.2 必须保留的 Package surface

- `validate_public_instance_surface` 保持；
- 独立 `validate_public_instance_method_surface` 继续检查完整namespace、method ID、callable link与
  `ImplMethod` kind，不依赖任何service selection；
- positive test确认`worker`的`run/stop`完整methods同时存在于Local ABI、`callableLinks`和
  `boundaryProjections`；
- `service_call_refs` 仍是required PackageArtifact wire member、build preimage member和独立
  fail-closed validator输入；
- positive test构造完整contract/service requirement + call ref，negative test分别删除ref和伪造slot，
  两者均拒绝。

### 3.3 Service authoring

`ServiceManifestAuthoring`新增：

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub service_calls: Vec<String>
```

wire name由既有`camelCase`规则精确生成`serviceCalls`。测试确认：

- missing与`serviceCalls: []`解析为相等的zero selection；
- 空selection序列化时省略；
- non-empty数组按原字符串保留；
- 此共享DTO层不解析public path，path shape/duplicate留给后续compiler parser；
- unknown `serviceCallRoots`仍拒绝。

### 3.4 Deployment input

`ServiceDeploymentOperationInput`现在精确为：

```rust
pub contract_operation_id: ContractOperationId,
pub package_callable_id: PackageCallableId,
```

测试确认canonical `packageCallableId` round-trip；旧`packagePublicPath`、新旧同时出现、missing exact ID
及其它unknown field全部拒绝。artifact-identity input validation同步检查非空
`packageCallableId`。最终`DeploymentOperationBinding`、`ServiceDeployment`和deployment identity未改。

### 3.5 Single-source checker

- `ServiceDeploymentOperationInput.requiredFields`改为
  `contract_operation_id + package_callable_id`；
- embedded canonical fixture同步 exact ID；
- self-test新增旧`package_public_path`拒绝矩阵；
- self-test新增第二个`ServiceDeploymentOperationInput` owner拒绝矩阵；
- 专用Node test执行checker `--self-test`，没有把零测试或默认全仓checker冒充本节点通过。

## 4. Test discovery 与执行

按任务要求，先执行四个`-- --list`，再运行同一selector。

| selector | `-- --list`实际选择 | 执行 |
| --- | ---: | --- |
| `skiff-artifact-model package_artifact` | 5 | 5 passed |
| `skiff-artifact-model ecosystem_authoring` | 5 | 5 passed |
| `skiff-artifact-model deployment` | 2 | 2 passed |
| `skiff-artifact-identity package_artifact` | 18 lib tests | 18 passed |
| Node single-source checker test | 1 | 1 passed |

identity命令还枚举了三个被filter后为0的其它test target；它们没有计入18，也没有被用作通过证据。
实际有效Rust测试合计30。

执行命令：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-artifact-model package_artifact -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-artifact-model ecosystem_authoring -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-artifact-model deployment -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-artifact-identity package_artifact -- --list

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

结果：

- Rust：`30 passed / 0 failed`；
- Node：`1 passed / 0 failed`；
- fmt：PASS，无输出；
- diff check：PASS，无输出。

没有运行workspace/full isolated/stable/live、instance、生态publish或外部服务。

## 5. 反向搜索

### 5.1 本节点共享owner

```text
rg '\bPackageServiceCallRoot\b|\bservice_call_roots\b' artifact-model artifact-identity
=> 0 files

rg -l '\bservice_call_refs\b' artifact-model artifact-identity
=> 8 files
```

`serviceCallRoots`在共享owner中的剩余字符串都只位于strict negative/absence tests，不是类型、字段、
validator或compat reader。全仓非文档`service_call_refs/serviceCallRefs`仍有52个文件，证明caller
call-site链没有被selection删除波及。

### 5.2 明确保留给下游DAG的consumer break

精确roots Rust identifier仍存在于24个范围外文件，按owner分布为：

- compiler：contract reader、pipeline/projection/emission/source及fixtures；
- deployment/store：4个fixture/storage/projection test文件；
- runtime：loader/linker/eval/host/package-test的11个fixture/helper文件。

旧deployment path的exact writer/reader仍有5个文件：

```text
compiler/driver/generated_deployment.rs
deployment/src/fixtures.rs
deployment/src/projection/operations.rs
deployment/src/projection/tests.rs
test-runner/src/ecosystem_smoke_fixture.rs
```

旧PackageArtifact v7/build v8 generation仍有9个下游文件：

```text
compiler/driver/authoring/package_publication/tests.rs
compiler/driver/authoring/tests.rs
compiler/projection-input/src/lib.rs
compiler/projection/src/package_artifact/tests/projection.rs
compiler/source/src/type_resolution_model.rs
compiler/tests/builtin_canonical_spelling.rs
router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts
router/tests/compilerGeneratedManifestCompatibility.test.ts
router/tests/filesystem-runtime-assembly-snapshot-loader.test.ts
```

`ServiceManifestAuthoring`的三个compiler struct literal也尚未补新字段。以上全部是任务明确允许的
checkpoint break，分别交给F408 producer/parser、F410 deployment consumer、F411
runtime/router/test fixtures；本节点没有迁移或加fallback。

默认运行`node scripts/check-artifact-identity-single-source.mjs`的额外诊断仍报告14个本节点范围外、
未改规则/文件上的既有全仓registry drift；其中已不包含
`ServiceDeploymentOperationInput`字段或第二owner错误。本任务规定的专用self-test为1/1 PASS，
没有把该额外诊断声明为green。

## 6. 自验收矩阵

| 任务条款 | 代码证据 | 测试/搜索证据 | 结论 |
| --- | --- | --- | --- |
| PackageArtifact v8无selection | `artifact-model/src/package_artifact.rs`、`schema.rs` | v8 round-trip；旧`serviceCallRoots`拒绝；共享exact identifier为0 | PASS |
| build v9 preimage无selection | `artifact-identity/src/package_artifact{,/projection}.rs`、`constants.rs` | v7 marker/v9 prefix与preimage absence test | PASS |
| Local ABI generation/preimage不变 | local ABI projection未改；marker v4/prefix v6 | identity test显式断言原generation | PASS |
| public instance完整surface保留 | `validation/public_instances.rs`及独立method surface validator | Local ABI/links/boundary positive + forgery negatives | PASS |
| `service_call_refs`保留 | model、projection、`validate_service_calls` | required wire、positive identity、missing/forged negatives；52-file全仓反搜 | PASS |
| manifest missing/empty规则 | `ServiceManifestAuthoring.service_calls` | missing/empty equality、skip-empty、unparsed string test | PASS |
| deployment input v3 exact ID | `artifact-model/src/deployment.rs`、identity input validation、schema | exact round-trip；old/missing/unknown拒绝 | PASS |
| final deployment不变 | `DeploymentOperationBinding`、`ServiceDeployment`、deployment constants未改 | scoped diff与single-source registry | PASS |
| forged/missing identities fail closed | PackageArtifact strict model与identity admission | missing build/local ID；stale v7/v8-domain identity拒绝 | PASS |
| checker继续唯一owner | checker required fields + self-test cases | Node 1/1 | PASS |
| 不迁移下游consumer | diff ownership与反向搜索 | 24 roots、5 path、9 generation下游文件仍显式暴露 | PASS |

结论：P5-F407共享N0 checkpoint完成；只解除任务声明的后继节点，不宣称整个生态已编译或成为稳定候选。
