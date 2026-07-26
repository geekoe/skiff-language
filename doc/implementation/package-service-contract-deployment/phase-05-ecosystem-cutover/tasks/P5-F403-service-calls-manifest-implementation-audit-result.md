# P5-F403 Service-calls manifest implementation audit result

状态：Complete。

## 1. 判定

```text
TASK_EXECUTABLE
```

F402 已经给出足够完整且互相一致的语义，未发现需要用户补充决定的设计空洞。实现必须按以下唯一方向
切换：

```text
api.yml
  -> 完整 Package public graph
  -> PackageArtifact（不含任何 service selection）

service.yml.serviceCalls
  + PackageArtifact.packageLocalAbi.publicSymbols
  + PackageArtifact.boundaryProjections
  -> typed selected operations
  -> ServiceContract + exact PackageCallableId bindings
  -> ServiceDeployment
```

不得保留 `api.yml serviceCall` 兼容，不得在 `PackageArtifact` 中换名保存 selection，不得在
deployment 再按 public-path 字符串反查 callable，也不得恢复 service-only compiler source owner。

本节点只新增本文，没有修改 production/test，没有访问 stable/live/外部服务，没有 merge/rebase/push。

## 2. 审计锚点

### 2.1 Skiff candidate

| 项 | 值 |
| --- | --- |
| candidate commit | `e17b543ef4bcaf936baafe7f9ed311c10d9a5fe7` |
| candidate tree | `6496128f13a9304f008de7b035de7465ab41054c` |
| parent | `a0549aa3fee88b7ec12d4c29a00b3c4e0e683401` |
| subject | `docs: select service calls from service manifest` |
| author date | `2026-07-26T20:09:49+08:00` |

审计 worktree HEAD 是 `a95344a1f6ca022092d08aaf99e62592e8c79890`，tree
`5dbf4458dcf27e3180b04ba0e0b79d8c654397b0`。相对 candidate 只多
`P5-F403-service-calls-manifest-implementation-audit.md`，production/test tree 与 candidate 相同。

### 2.2 只读 ecosystem snapshots

| root | commit | tree | 状态 |
| --- | --- | --- | --- |
| `/Users/geek/workspace/internals` | `5861c13f3a92b7fb56a5cfa689e46f5d0462a02d` | `867c99c155386299e7dbb8b4fed95cee2427ba84` | clean |
| `/Users/geek/workspace/internals-phase-05-integration` | `3a7234610c53b11c5f2cfdb5b04448408e924e31` | `72901006d82f6abafef3efb6567e4ccba6aa4caf` | clean |
| `/Users/geek/workspace/skiff-packages` | `5defc94161cee14def1a6bbb340308004e65b741` | `d8763acf82e0320135704297f2419bf5cd3558e5` | clean |
| `/Users/geek/workspace/skiff-packages-phase-05-integration` | `3653a294cfb92e60e220dcccc94bc8e8add65b33` | `93602c0e99ef15cf539334931e522d5ba844871c` | clean |

## 3. 当前 `api.yml serviceCall` 完整 producer/consumer 链

### 3.1 Parser 到 PackageArtifact writer

| 顺序 | owner / symbol | 当前行为 | 实现动作 |
| --- | --- | --- | --- |
| 1 | `compiler/input/src/api_yml.rs::{api_function_leaf, parse_function_leaf, parse_public_instance_leaf}` | mapping leaf 识别 `serviceCall`；function mapping 强制 `source + serviceCall:true`，public instance 可选 marker；scalar function 默认为 false | 删除 marker grammar；function 只剩 scalar source selector，或若仍保留 mapping leaf则只能表达 Package-owned 字段，不能接受 `serviceCall`；public instance 只接受 `const/interfaces` |
| 2 | `compiler/core/src/api_spec.rs::{PublicationApiEntry, PublicationApiPublicInstanceEntry, with_service_call}` | DTO 保存 `service_call: bool` | 删除两个字段与 builder；`compiler/input/src/api_spec.rs` 只是 re-export，同步编译 |
| 3 | `compiler/source/src/api.rs::{PublicationApi::build_from_publication_sources_with_resolved_modules, PublicCallable, PublicInstance}` | source resolution 验证 marked non-function，并把 bool 写入 resolved public graph | 删除 marker 分支与 resolved DTO 字段；function/public-instance 的 Package public graph 验证保留 |
| 4 | `compiler/source/src/api_seed.rs::PublicationApiSeed::from_publication_api` | clone `PublicCallable` / `PublicInstance`，因此隐式携带 bool | clone graph 保留，但类型不再有 selection |
| 5 | `compiler/source/src/compile_model.rs::{ExportCallableBinding, ExportPublicInstanceBinding, ExportBindingModel::from_publication_api}` | 再复制 bool 到 export binding | 删除 selection 字段/复制 |
| 6 | `compiler/compiled/src/projection_input.rs::{publication_api_seed_projection, build_export_bindings, public_callable_projection, public_instance_projection}` | 把 bool 同时写入 seed projection 与 export projection | 两条 handoff 都删除 selection |
| 7 | `compiler/projection-input/src/lib.rs::{ExportCallableProjection, ExportPublicInstanceProjection, PublicCallableProjection, PublicInstanceProjection}` | terminal projection DTO 的四个字段 | 删除四个字段及 JSON/test fixture |
| 8 | `compiler/projection/src/package_artifact/api_exports.rs::{PackageExports.service_call_functions, PackageExportPublicInstance.service_call, project_package_exports}` | 过滤 marked function，并保存 instance bool | 删除两个 selection 字段及过滤；`PackageExports` 只描述完整 Package exports |
| 9 | `compiler/projection/src/package_artifact/projection.rs::project_service_call_roots` | 从 Package exports + Local ABI 写 `PackageServiceCallRoot::{Function,PublicInstance}` | 删除函数、调用与 artifact field assignment |
| 10 | `compiler/driver/pipeline/mod.rs::{compile_package_with_service_call_roots, service_call_paths, compile_service_package}` | ordinary package 遇 marker 报错；service role 才允许 writer；随后从 artifact roots 投影 contract | 删除 role/marker gate；`compile_service_package` 先走普通 `compile_package`，再把 typed manifest selection 传给 contract projection |
| 11 | `compiler/driver/authoring.rs::{build_authoring_object, generate_service_deployment}` | terminal coordinator读取同一 `ServicePackageRoot`，消费 compiled service API并生成deployment | 保持单一 coordinator；让 manifest selection只传给contract一次，deployment只接 exact typed mapping |

`compiler/source/src/contract_type_resolution/callables.rs` 的 `service_call:false` 与
`compiler/projection/src/package_artifact/export_links/tests/fixtures.rs` 等剩余赋值只是上述 DTO 的测试构造，
必须随类型删除。`compiler/core/src/spawn_targets.rs` 中的 `service_call_ref` 是调用点引用，不属于 selection。

### 3.2 当前 schema 与 boundary generation

selection 删除后不需要扩大 Package generation：

- `compiler/projection/src/package_artifact/schema.rs::project_package_schema` 已从完整 Package public type
  exports 生成所有 eligible named schema records，不读取 roots。
- `compiler/projection/src/package_artifact/callables/mod.rs::project_package_callable_surface` 已遍历完整
  Package public callable surface，生成 `public_symbols`、`callable_links`、
  `callable_semantic_facts` 与每个 callable 的 `boundary_projections`，不只处理 selected roots。
- contract 的 schema closure 当前在
  `compiler/contract/src/projection.rs::{collect_operation_refs, transitive_closure}` 从 selected operation
  boundary 开始收敛；这个 owner 保留，只替换 selection 输入。

因此实现不是“先按 manifest 补生成 Package boundary”，而是“从已经完整生成的 Package surface 做
service-owned typed selection”。

## 4. `PackageArtifact.serviceCallRoots` 全部 owner 与 reader

### 4.1 Canonical model、identity、validation

| 类别 | owner / symbol | 同步结论 |
| --- | --- | --- |
| wire model | `artifact-model/src/package_artifact.rs::{PackageServiceCallRoot, PackageArtifact.service_call_roots}` | 删除 enum、field、`public_path()`；strict serde 新 generation 不接受旧 `serviceCallRoots` |
| public export | `artifact-model/src/lib.rs` | 删除 `PackageServiceCallRoot` re-export |
| schema generation | `artifact-model/src/schema.rs::PACKAGE_ARTIFACT_SCHEMA_VERSION` | `skiff-package-artifact-v7` 必须升为 v8 |
| build preimage type | `artifact-identity/src/package_artifact.rs::PackageArtifactBuildIdentityProjection.service_call_roots` | 删除 preimage field；Local ABI projection无此字段 |
| build preimage assembly | `artifact-identity/src/package_artifact/projection.rs::build_projection_from_validated` | 删除 clone/sort/write roots |
| validation | `artifact-identity/src/package_artifact/validation.rs::{validate_package_artifact_surface, validate_service_call_roots}` | 删除 roots validator/call；保留独立的 `validate_public_instance_surface`、callable link、boundary 与 `service_call_refs` validation |
| identity constants | `artifact-identity/src/constants.rs` | build preimage shape 变化：marker v6→v7，build prefix v8→v9；Local ABI marker v4/prefix v6 不变 |
| identity tests | `artifact-identity/src/package_artifact.rs` 与 `package_artifact/public_instance_tests{,/fixtures}.rs` | 删除 roots identity/forgery tests，改为证明 manifest selection 不进入 Package identity；public-instance完整 surface tests 保留 |

当前 `PackageArtifact` strict wire 没有 `#[serde(default)]`，测试
`artifact-model/src/package_artifact.rs::package_artifact_wire_rejects_legacy_aggregate_fields`
还显式证明缺少 `serviceCallRoots` 会失败。因此只删 Rust field 而不升 schema，会让同一个 v7 名称拥有两种
互斥 shape；这是不允许的。

### 4.2 Production readers

| 域 | 当前 production reader | 是否读 roots 语义 | 同步动作 |
| --- | --- | --- | --- |
| compiler contract | `compiler/contract/src/projection.rs::{project_service_api, selected_service_callables}` | 是，唯一 semantic reader | 删除 roots reader，改读 typed `service.yml` selection |
| compiler emission | `compiler/emission/src/emission/package_artifact.rs::{materialize_package_artifact, package_artifact_json}` | generic serde + identity validation | 随 model/generation 同步；无 selection 逻辑 |
| canonical store | `deployment/src/storage/records.rs::{write_package_artifact, read_package_artifact, resolve_package_artifact_schema}` | generic serde + identity validation | 随 model/generation 同步；test literals 删除 field |
| deployment projection | `deployment/src/projection/**` | 不读 roots；读 Local ABI/boundary 与 deployment input | 不得新增 roots reader；改 exact callable input 见第 6 节 |
| runtime loader | `runtime/loader/src/runtime_assembly/content_validation.rs::validate_package_ref` 与 `runtime_assembly.rs` | 不读 roots；整体调用 `validate_package_artifact_identities` | 新 v8/v9 可 admission；test literal 删除 field |
| runtime linker/eval/host | linker 只消费 deployment exact binding、callable links 与 File IR | production 无 roots reader | 只删/改 test literals；不得把 selection 下推 runtime |
| router production | `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts` raw-load PackageArtifact files | 不读 roots，但硬编码 `skiff-package-build-v8:sha256:` | build generation 升级时改为 v9；不能只改 Rust constant |
| router compatibility | `router/tests/compilerGeneratedManifestCompatibility.test.ts` | 断言 v7/v8 与 `serviceCallRoots:[]` | 改断言 v8/v9 且 field absent；`filesystem-runtime-assembly-snapshot-loader.test.ts` 的 v8 fixture 同步 |

runtime 的 `service_call_refs`、File IR `kind: serviceCall`、service requirement slot 与 linker 的
activation-relative resolution 是 consumer call-site 链，必须完整保留。它们与“provider选择哪些public
roots成为ServiceContract operation”是两个不同概念。

### 4.3 所有 roots literal/test/checker 同步集合

对 candidate 执行 exact reverse search 得到 35 个非文档文件。除上述 production owners 外，以下都是
必须随新 model 编译同步的 fixture/literal：

- artifact：
  `artifact-identity/src/package_artifact.rs`、
  `artifact-identity/src/package_artifact/public_instance_tests.rs`、
  `artifact-identity/src/package_artifact/public_instance_tests/fixtures.rs`、
  `artifact-model/src/package_artifact.rs`。
- compiler：
  `compiler/contract/src/projection.rs`、
  `compiler/driver/authoring/tests.rs`、
  `compiler/driver/pipeline/tests.rs`、
  `compiler/driver/source_compile/canonical_dependencies.rs`、
  `compiler/emission/src/emission/package_artifact/tests.rs`、
  `compiler/projection-input/src/lib.rs`、
  `compiler/projection/src/package_artifact/projection.rs`、
  `compiler/projection/src/package_artifact/tests/projection.rs`、
  `compiler/source/src/type_resolution_model.rs`、
  `compiler/tests/artifact_model_conformance.rs`、
  `compiler/tests/service_call_roots.rs`。
- deployment/store：
  `deployment/src/assembly/tests/fixtures.rs`、
  `deployment/src/projection/tests.rs`、
  `deployment/src/storage/records.rs`（`cfg(test)` fixture）、
  `deployment/src/storage/tests.rs`。
- runtime：
  `runtime/eval/src/assembly_execution/ordinary/tests.rs`、
  `runtime/eval/src/assembly_execution/ordinary/tests/service_error_consumer.rs`、
  `runtime/eval/src/assembly_execution/projection.rs`（test projection）、
  `runtime/eval/src/assembly_execution/service_error_channel/tests.rs`、
  `runtime/eval/src/spawn_ops/canonical_tests.rs`、
  `runtime/host/src/loader/assembly_admission/tests/execution/artifacts.rs`、
  `runtime/host/src/loader/assembly_admission/tests/full_chain.rs`、
  `runtime/linker/src/assembly/tests/fixtures.rs`、
  `runtime/linker/src/assembly_execution/service_error_index.rs`（test helper）、
  `runtime/loader/src/runtime_assembly/tests.rs`、
  `runtime/package-test/tests/support/mod.rs`。
- router：
  `router/tests/compilerGeneratedManifestCompatibility.test.ts`。

`scripts/check-artifact-identity-single-source.mjs` 当前没有枚举
`PackageArtifact.service_call_roots`，但它锁定 canonical model/identity owner，且精确锁定
`ServiceDeploymentOperationInput` fields；后者从 `package_public_path` 改为
`package_callable_id` 时必须同步 checker 的 `requiredFields` 与 embedded fixture。

另外，以下 hard-coded generation/golden 也必须同步：

- `artifact-model/src/schema.rs` 与 `artifact-model/src/package_artifact.rs`；
- `artifact-identity/src/{constants.rs,tests/mod.rs,package_artifact.rs}`；
- `compiler/driver/authoring/{tests.rs,package_publication/tests.rs}`；
- `compiler/projection-input/src/lib.rs`；
- `compiler/projection/src/package_artifact/tests/projection.rs`；
- `compiler/source/src/type_resolution_model.rs`；
- `compiler/tests/{artifact_model_conformance.rs,builtin_canonical_spelling.rs}`；
- `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts`；
- `router/tests/{compilerGeneratedManifestCompatibility.test.ts,filesystem-runtime-assembly-snapshot-loader.test.ts}`。

## 5. `service.yml.serviceCalls` strict DTO、parser 与 typed selection

### 5.1 DTO 与 parser

canonical owner 是
`artifact-model/src/ecosystem_authoring.rs::ServiceManifestAuthoring`，它已有
`#[serde(rename_all = "camelCase", deny_unknown_fields)]`。增加：

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub service_calls: Vec<String>,
```

这样 missing 与 `[]` 都表示 zero-operation service；因为 Skiff 未发布，不增加 alias 或旧 marker
fallback。

`compiler/input/src/service_config.rs::{read_service_package_root, read_service_manifest}` 保持 canonical
Package+Service root 要求，并在 manifest parse 后：

1. 每个元素必须是非空、canonical dotted Package public path；复用
   `skiff_compiler_core::export_config::is_valid_dotted_module_path` 的同一语法事实来源。
2. duplicate path 直接 fail closed，不能在 sort/dedup 时静默吞掉。
3. parser 只验证字符串 shape 与 duplicate；unknown、non-callable、instance method、
   boundary-unavailable 必须在拥有完整 typed Package surface 的 contract projection 验证。
4. `ServiceManifestAuthoring` 的 struct literals 位于
   `artifact-model/src/ecosystem_authoring.rs`、`compiler/input/src/service_config.rs`、
   `compiler/tests/generated_service_deployment.rs` 与
   `compiler/tests/http_gateway_projection.rs`，全部增加新字段或使用 constructor。

### 5.2 Typed selection owner

唯一 owner 应位于 `compiler/contract`，紧邻
`project_service_api`，而不是 artifact-model、projection writer、deployment 或 runtime。建议内部
typed seam：

```text
ServiceCallSelection
  roots: canonical sorted public root paths
  operations:
    stable operation public path
      -> exact PackageCallableId
      -> exact BoundaryCallableProjection
```

解析规则：

- `PackageLocalAbiSymbol::Callable` 且不是任一 public-instance method：选一个 operation；
- `PackageLocalAbiSymbol::PublicInstance`：按其 `methods: BTreeMap<method, PackageCallableId>`
  展开全部 listed-interface methods，operation stable key 是 `root.method`；
- 直接选择 `root.method` 必须拒绝，即使该 path 同时是 public callable；
- `Type`、`Constant`、unknown path 拒绝；
- 两个 roots 最终落到同一 callable 拒绝；
- 每个 exact callable 必须有 `BoundaryCallableProjection::Available`；所有 unavailable roots/reasons
  聚合报告；
- 输入 roots 先验证 duplicate，再 canonical sort；数组顺序不进入任何 identity。

当前 `ServiceApiProjection.available: BTreeMap<String, PackageCallableId>` 已经是可复用的 typed
operation→exact callable handoff。可以保留它，但其来源改为 `ServiceCallSelection`，不能从 artifact
roots 重建。

`compiler/driver/pipeline/mod.rs::compile_service_package` 应调用普通 `compile_package`，然后：

```text
project_service_api(
  service id,
  service_root.service.service_calls,
  PackageArtifact,
  resolved PackageSchema records
)
```

`compile_package_with_service_call_roots`、`validated_service_root` 与 `service_call_paths` 整体删除。

## 6. Contract → deployment 当前流与目标单一流

### 6.1 当前重复解析

1. `compiler/contract/src/projection.rs::selected_service_callables` 读
   `PackageArtifact.service_call_roots`，把 instance 展开为 `root.method`。
2. `project_service_api` 读取 boundary，生成 operations/schema closure，并在
   `ServiceApiProjection.available` 保存 exact `PackageCallableId`。
3. `compiler/driver/generated_deployment.rs::operation_bindings` 已经拿到 exact callable，却反向扫描
   `PackageLocalAbi.public_symbols` 恢复一个 `package_public_path`。
4. 它写入
   `artifact-model/src/deployment.rs::ServiceDeploymentOperationInput.package_public_path`。
5. `deployment/src/projection/operations.rs::project_operation_bindings` 又按该字符串查 Local ABI，第二次
   猜 callable，之后才验证 boundary/contract/facts，并输出 exact
   `DeploymentOperationBinding.package_callable_id`。

第 3–5 步违反 F402 的“tooling 从被选择 public root 的 exact PackageCallableId 确定性生成 binding”。

### 6.2 目标流

```text
api.yml + source
  -> full Package public symbols/callable links/boundary projections
  -> PackageArtifact v8

service.yml.serviceCalls
  -> compiler/contract typed root resolution（唯一字符串解析点）
  -> selected operation stable key + exact PackageCallableId + boundary
  -> ServiceContract v4 + ServiceApiProjection.available

ServiceApiProjection.available
  -> compiler/driver generated ServiceDeploymentOperationInput {
       contractOperationId,
       packageCallableId
     }
  -> deployment validates exact callable exists, boundary/facts/descriptor match
  -> ServiceDeployment v2 DeploymentOperationBinding
```

具体变化：

- `ServiceDeploymentOperationInput.package_public_path: String` 改为
  `package_callable_id: PackageCallableId`。
- `compiler/driver/generated_deployment.rs::operation_bindings` 直接 clone
  `ServiceApiProjection.available` 的 ID，删除 reverse scan。
- `deployment/src/projection/operations.rs` 用 exact ID 查
  `boundary_projections`、`callable_semantic_facts` 与 `callable_links`；同时确认该 ID 是 implementation
  Local ABI 的 public callable或public-instance method。不得尝试 path fallback。
- forged/unknown ID、contract mismatch、boundary unavailable、facts mismatch继续结构化 fail closed。

这改变 `ServiceDeploymentInput` wire shape，所以 input schema
`skiff-service-deployment-input-v2` 必须升 v3。最终 `ServiceDeployment` 已经只存 exact callable ID，
output shape 与 identity projection不变，仍是 v2。

## 7. Generation 与 identity 结论

### 7.1 当前 preimage owners

| identity | current owner / preimage | service selection 当前影响 |
| --- | --- | --- |
| Package build | `artifact-identity/src/package_artifact.rs::PackageArtifactBuildIdentityProjection` + `package_artifact/projection.rs::build_projection_from_validated`；marker `skiff-package-artifact-build-identity-v6`，prefix `skiff-package-build-v8:sha256` | 当前错误地包含 sorted `service_call_roots` |
| PackageLocalAbi | `PackageArtifactLocalAbiIdentityProjection` + `local_abi_projection`；只含 marker v4、package id、`public_symbols`；prefix v6 | roots 不影响 |
| ServiceProtocolIdentity | `artifact-identity/src/contract.rs::service_protocol_identity_projection`；含 marker v4、service id、operations、reachable `package_type_requirements`；prefix v4 | operation集合/descriptor变化才影响，contract version label不影响 |
| contract operation ID | `artifact-identity/src/contract.rs::contract_operation_id`；service id + stable operation key | selected operation stable key或service id变化影响 |
| generated deployment revision | `compiler/driver/generated_deployment.rs::generated_revision`；当前 hash `(service.id, profile_name, implementation.package_build_id, profile, entire service manifest)` | 新字段若直接进入会错误地让数组 reorder 改 revision |
| DeploymentArtifactIdentity | `artifact-identity/src/deployment.rs::service_deployment_identity_projection`；含 normalized contract ref、revision、implementation ref、exact operation/package/service/gateway/config/policy bindings | operation binding、protocol ref、revision、implementation变化影响 |

### 7.2 必须采用的 generation

| surface | candidate | target | 原因 |
| --- | --- | --- | --- |
| PackageArtifact wire | v7 | v8 | 删除 required `serviceCallRoots` |
| Package build preimage marker | v6 | v7 | 删除 canonical preimage field |
| Package build identity prefix | v8 | v9 | preimage generation变化，不能让同 prefix 表示两种算法 |
| PackageLocalAbi marker/prefix | v4 / v6 | 不变 | preimage bit-identical |
| ServiceContract / protocol | contract v4 / protocol v4 | 不变 | output shape与identity算法不变 |
| ServiceDeploymentInput | v2 | v3 | operation input从 public path改 exact callable ID |
| ServiceDeployment / deployment identity | v2 / v2 | 不变 | canonical output本来就是 exact callable ID |
| File IR、Package schema index/type identities | current | 不变 | selection不进入这些 surfaces |

v7 artifact 即使 roots 为空，其 build preimage也含 `"serviceCallRoots":[]`；v8 删除该 member。因此切代时
相同 source 的 build ID 会跨 generation 变化，这是 schema migration，不违反 F402 所说的“在新模型内只
改变 `serviceCalls` 时 PackageArtifact bit-identical”。

### 7.3 Revision order normalization

`generated_revision` 不能继续无处理地 hash 新 `ServiceManifestAuthoring`。必须在 duplicate 已拒绝后，
对 revision 使用显式 projection，其中 `serviceCalls` canonical sort，或使用
`ServiceCallSelection.roots`/typed operations；不得 hash 原始数组顺序。implementation build ID 已覆盖
Package implementation，deployment canonical identity又覆盖 exact bindings，因此无需把字符串重新解析。

### 7.4 正反例矩阵

| 变化 | PackageArtifact/build | Local ABI | ServiceProtocol | deployment revision / identity |
| --- | --- | --- | --- | --- |
| `serviceCalls: [a,b]` 改 `[b,a]` | 相同 | 相同 | 相同 | 必须相同；这是 revision normalization 的正例 |
| selection `{a}` 改 `{a,b}`，其它不变 | 相同 | 相同 | operations变化，因此变化 | bindings/revision/identity变化 |
| missing 改 `[]` | 相同 | 相同 | 相同 zero-operation protocol | 相同 |
| duplicate、unknown、non-callable、instance method、boundary unavailable | Package编译值与selection无关；整体service pipeline不得发布receipt | 不因selection变化 | fail closed，无 protocol receipt | 无 deployment receipt |
| 只改 service id | 相同 | 相同 | operation IDs/protocol变化 | revision/deployment identity变化 |
| selected callable body变化，boundary不变 | build变化 | signature不变则Local ABI相同 | 相同 | implementation ref/revision/deployment identity变化 |
| unselected public callable boundary或body变化 | build变化；public signature变时Local ABI也变化 | 依变更而定 | selected contract可保持相同 | implementation ref/revision/deployment identity变化 |
| `api.yml` source mapping/public graph变化 | build变化，Local ABI通常变化 | 依public surface而定 | selected boundary/stable keys不变时可相同 | exact implementation/binding或revision变化 |
| 同一 source 同时被 `serviceCalls` 与 HTTP handler引用 | Package只生成一次完整 callable | 同一Package surface | service operation identity | 独立 gateway identity；二者不得相等或互推 |

## 8. Source/fixture/ecosystem 迁移矩阵

### 8.1 Candidate 中的 marker 与 Rust fixtures

candidate 的真实 `**/api.yml` 没有 `serviceCall:true`。marker 只出现在 6 个 test/parser source 文件：

| owner | 当前选择 | 迁移 |
| --- | --- | --- |
| `compiler/input/src/api_yml.rs` tests | function/public-instance marker正负 parser cases | `api.yml` marker全部成为 unsupported-field negative；在 `service_config` 增加 missing/empty、dotted、duplicate、wrong-shape parser cases |
| `compiler/tests/generated_service_deployment.rs` | ordinary function `read` | `api.yml` 改 scalar；fixture `service.yml` 加 `serviceCalls: [read]` |
| `compiler/tests/http_gateway_projection.rs` | ordinary function `dual`，同时是 HTTP handler | `api.yml` 改 scalar；`service.yml` 加 `[dual]`；保留 service/gateway identity 分离断言 |
| `compiler/tests/service_call_roots.rs` | ordinary `selected`、public instance `worker`（含 generic receiver） | 重命名为 manifest-selection语义；`service.yml` 按 case 加 `[selected]`、`[worker]` 或两者；新增 method-path rejection |
| `compiler/tests/service_conformance.rs` | stream function `events` | `api.yml` scalar；`service.yml` 加 `[events]` |
| `scripts/tests/package-service-authoring.test.mjs` | ordinary function `ping` | `api.yml` scalar；`service.yml` 加 `[ping]` |

所有 `service_call: bool` DTO constructors 与第 4.3 节的 `service_call_roots` literals 一并机械迁移。
`service_call_refs` fixtures不删除。

### 8.2 Candidate canonical service fixtures

| root | 当前形态 | `serviceCalls` 目标 |
| --- | --- | --- |
| `compiler/tests/fixtures/router-websocket-fixture` | canonical package+api+service；只有 external HTTP，API `{}` | missing/`[]`，zero operation |
| `test-runner/fixtures/alias-return-catch-once-tests` | canonical test service，API `{}` | missing/`[]` |
| `test-runner/fixtures/package-service-host/provider` | canonical provider；public function `echo` | `[echo]`；consumer 的 `payments/echo` 是真实 inbound service call |
| `test-runner/fixtures/package-service-host/consumer` | canonical package+service；public `owner/run`，自身只消费 payments | missing/`[]`；测试通过 package subject访问它，不等于对外service operation |
| `test-runner/fixtures/package-service-host/consumer-tests` | canonical test service，API `{}` | missing/`[]` |
| `test-services/std` | canonical test service，API `{}` | missing/`[]` |

### 8.3 Candidate 中仍是 service-only 的 current sources

这些不是第二种合法 compiler root，必须迁到 Package+Service root：

| root | 当前 controls | selection |
| --- | --- | --- |
| `runtime/encrypted-storage-live/default-service` | `.skiff + service.yml`，无 `package.yml/api.yml`；legacy route-list HTTP | canonicalize package/API/service；没有 service consumer证据，`serviceCalls: []`，external ingress独立迁移 |
| `runtime/encrypted-storage-live/mapped-service` | 同上，另有 legacy service-owned packages | dependencies迁回 `package.yml`；`serviceCalls: []` |
| `runtime/live-tests` | `.skiff + service.yml`，无 `package.yml/api.yml`；legacy package route handler | canonicalize；`serviceCalls: []` |

本节点不扩展审计去解决这些 legacy HTTP/dependency shape；这里只把它们列为 source-root migration，
禁止以它们为理由恢复 service-only input owner。

### 8.4 Skiff Packages

`/Users/geek/workspace/skiff-packages` main 没有 service root/marker。

`/Users/geek/workspace/skiff-packages-phase-05-integration/registry/api.yml` 是四个 ecosystem snapshot
中唯一真实 YAML marker source，共 20 个普通 function。全部改为 scalar API entry，并把以下完整 public
paths 移到 `registry/service.yml.serviceCalls`：

```text
packageArtifactPut
packageArtifactRead
packageArtifactPointerRead
packageArtifactPointerCas
packageArtifactPointerHistory
serviceContractPut
serviceContractRead
serviceContractPointerRead
serviceContractPointerCas
serviceContractPointerHistory
serviceDeploymentPut
serviceDeploymentRead
serviceDeploymentPointerRead
serviceDeploymentPointerCas
serviceDeploymentPointerHistory
runtimeAssemblyPut
runtimeAssemblyRead
runtimeAssemblyPointerRead
runtimeAssemblyPointerCas
runtimeAssemblyPointerHistory
```

同仓库 `tests/{aliyunoss,http-session,openai-live,openai,registry,track}` 都是 `kind:test`、API `{}`，
保持 zero operation。`tests/registry` 通过 package/topLevel 依赖测试 registry，不是 service selection
owner。

### 8.5 Internals integration

integration 已有 canonical Package+Service roots，但没有 marker；selection 必须由真实 service
dependency calls补齐：

| service root | 类型 | 证据 | `serviceCalls` |
| --- | --- | --- | --- |
| `codex-relay/service` | public instance | `aihub/service/internal/aihub_service.skiff` 调用 `codexRelay/relayProxy.responsesCompletedResult` | `[relayProxy]`，展开 listed interface 的 `responsesCompleted` 与 `responsesCompletedResult` 全部 methods |
| `aihub/service` | 两个 public instances | Agine 调用 `aihub/managedLlm.{streamChat,webSearch}` 与 `aihub/providerCatalog.builtinProvider` | `[managedLlm, providerCatalog]`；完整展开为 `managedLlm.{validateChat,streamChat,webSearch}` 与 `providerCatalog.{builtinProvider,model}`，不能只写三个被调用的 method paths |
| `agine/service` | external HTTP/WebSocket handlers；未发现 inbound service consumer | 无 service requirement指向 Agine | missing/`[]` |
| `skiff-platform/account` | external HTTP handlers；未发现 inbound service consumer | `root.account.*` 只在本 package test 中，是 package/local访问 | missing/`[]` |

Agine/AIHub/Relay integration manifests 仍含另一个 Phase 节点负责的 legacy ingress shape；本 selection
迁移不得顺手泛化该问题。

### 8.6 Internals main canonicalization

main 中以下五个 current roots 都是 `api.yml + service.yml` 且缺 `package.yml`：

```text
agine/service
aihub/service
codex-relay/service
skiff-platform/account
skiff-platform/package-registry
```

前四个迁到 integration 已建立的同目录 Package+Service roots，并采用第 8.5 节 selection。
`skiff-platform/package-registry` 不得作为第二个 registry service复活；Phase integration 已由官方
`skiff-packages/registry` 取代，采用第 8.4 节 20 roots。compiler owner始终只接受：

```text
ordinary Package = .skiff + package.yml + api.yml
service          = 同一 Package root + service.yml + optional config.*.yml
```

### 8.7 文档 reverse search

canonical architecture/reference/overview 共 11 个匹配文件，已由 F402 更新为新设计；实现后只需确认没有
重新写回旧语义。`doc/implementation/**` 有 41 个匹配 task/result 文件，是历史证据，其中 F400/F400A
已明确 superseded。它们不属于 source migration，也不应做全局文本替换；新 acceptance 的 reverse
search 应把 canonical docs 与 historical evidence 分开报告。

## 9. 最小可执行 DAG 与唯一任务边界

共享 model checkpoint 会让尚未迁移的下游 crates 暂时无法编译；其 acceptance 只要求自己拥有的
artifact-model/artifact-identity/checker probes green。所有 consumer 从同一 checkpoint commit起步，
写入集合不得重叠。

### 9.1 DAG

```text
P5-F404 shared schema/model/identity checkpoint
  ├─ P5-F405 Package public-graph producer + service manifest parser
  │    └─ P5-F406 typed selection + contract/driver generation
  ├─ P5-F407 deployment exact-callable consumer
  └─ P5-F408 runtime/router/test-runner fixture generation sync

P5-F406 + P5-F407
  ├─ P5-F409 Skiff Packages source migration
  └─ P5-F410 Internals canonical service selection migration

P5-F404..P5-F410
  └─ P5-F411 integrated acceptance
```

### 9.2 建议 task files、owners 与最早风险探针

| task | prerequisites | 独占 write owners | 最早风险探针 | 验证 |
| --- | --- | --- | --- | --- |
| `P5-F404-service-calls-shared-schema-model-checkpoint.md` | F403 | `artifact-model/**`、`artifact-identity/**`、`scripts/check-artifact-identity-single-source.mjs` | v8 strict wire rejects old `serviceCallRoots`; v9 build preimage excludes selection; Local ABI unchanged；deployment input exact ID/v3 | artifact-model package tests、artifact-identity package tests、single-source checker |
| `P5-F405-package-public-graph-and-service-manifest-parser.md` | F404 | `compiler/{core,input,input-model,source,compiled,projection-input,projection,emission}/**` 及这些 crates 的局部 tests | `api.yml serviceCall` fail closed；missing/empty/dotted/duplicate serviceCalls；artifact没有selection field但完整 public instance/method surface仍存在 | input `api_yml` + `service_config`；projection/package artifact focused tests |
| `P5-F406-service-manifest-typed-contract-projection.md` | F405 | `compiler/contract/**`、`compiler/driver/**`、对应 `compiler/tests/{service_call_roots,generated_service_deployment,http_gateway_projection,service_conformance}.rs` | ordinary fn、public instance全展开、method path/unknown/noncallable/unavailable/duplicate拒绝、zero op、reorder稳定；driver不反查path，revision normalization | contract projection + generated deployment test binary |
| `P5-F407-deployment-exact-callable-binding.md` | F404；与 F405/F406 可并行开发，integration前等 F406 | `deployment/**` | forged/missing exact ID、non-public ID、boundary/facts/descriptor mismatch fail closed；无 path fallback | deployment projection focused tests |
| `P5-F408-runtime-router-test-fixture-generation-sync.md` | F404 | `runtime/{loader,linker,eval,host,package-test}/**`、`router/**`、`test-runner/**`、`test-services/**`、`scripts/tests/package-service-authoring.test.mjs` | loader接收 v8/v9；router v9 record path；provider fixture `[echo]`；所有 roots literals删除；call-site refs仍工作 | exact loader/linker tests、router compiler compatibility、package-service host focused test |
| `P5-F409-skiff-packages-service-calls-migration.md` | F406+F407 | `/Users/geek/workspace/skiff-packages-phase-05-integration/{registry,tests}/**` | registry 20 operations exact、API无 marker、test services zero op、reorder identity proof | isolated registry package/service build against fresh artifact root；不访问 stable/live |
| `P5-F410-internals-service-calls-migration.md` | F406+F407 | `/Users/geek/workspace/internals-phase-05-integration/{agine/service,aihub/service,codex-relay/service,skiff-platform/account}/**` | Relay instance 2 methods；AIHub selected instances覆盖真实calls；Agine/account zero op；无 service-only owner | isolated source/build probes；不启动常驻 web/stable |
| `P5-F411-service-calls-manifest-cutover-acceptance.md` | F404–F410 integrated | 只写 result，发现缺口另开 leaf | 全仓 marker/roots reverse search为允许的历史文本/`service_call_refs`；identity正反例；ecosystem exact operation sets | focused gates汇总；不得把 full workspace/live gate塞入本 DAG |

每个实现节点先用 `cargo test ... -- --list` 记录当时实际 selection count，再运行同一 selector；新增测试会
改变本文的 candidate count，不能复制一个过期数字冒充执行证据。

### 9.3 本审计实际 test selection

| 命令 | 实际选择 | 动作/结果 |
| --- | ---: | --- |
| `cargo test -p skiff-compiler-input api_yml -- --list` | 12 | list only |
| `cargo test -p skiff-compiler-input api_yml` | 12 | 12 passed |
| `cargo test -p skiff-compiler-input service_config -- --list` | 11 | list only |
| `cargo test -p skiff-compiler-input service_config` | 11 | 11 passed |
| `cargo test -p skiff-artifact-model package_artifact -- --list` | 5 | list only |
| `cargo test -p skiff-artifact-identity package_artifact -- --list` | 16 | list only；其它 targets 0 |
| `cargo test -p skiff-compiler-contract projection -- --list` | 4 | list only |
| `cargo test -p skiff-deployment projection -- --list` | 16 | list only |
| `cargo test -p skiff-compiler --test generated_service_deployment -- --list` | 10 | list only |

一次 discovery 命令误用不存在的 package id
`skiff-compiler-driver`，实际选择 0；改用 `skiff-compiler` 的 exact integration test target 后得到上表
10。另一次未限定 test target 的 `cargo test -p skiff-compiler generated_deployment -- --list` 在选择
前被 candidate 既有的 `compiler/tests/actor_dispatch_linking.rs` `global_ingress` compile error阻断，
实际执行 0；该 unrelated baseline error不扩展 F403/F404–F411 scope。

## 10. 证据失效边界

- candidate commit/tree变化，或 F402 canonical design再变：整份审计失效。
- `PackageArtifact` model/schema/build preimage任一变化：F404及所有 downstream receipts失效；
  Local ABI receipt必须单独重跑，不能由 build receipt代替。
- Package public graph/boundary generation变化：F405、F406、两个 ecosystem migration 与 acceptance失效；
  deployment exact-ID算法未变时 F407 unit evidence仍有效。
- selection expansion/validation、operation stable key 或 revision normalization变化：F406、F409、F410、
  F411失效；Package v8/v9 wire receipt不失效。
- `ServiceDeploymentOperationInput` 或 deployment validation变化：F404 model receipt、F407及所有
  end-to-end deployment receipts失效；parser-only evidence不失效。
- runtime/router只做 generation/literal同步；若有人新增 production roots reader，则本审计的
  “selection不下推runtime”结论被违反，必须先回到 F402，不得把它当普通 consumer patch。
- 单一 ecosystem fixture/source变化只使其对应 F409 或 F410 receipt失效，不应要求重做无关 ecosystem
  node。
- historical task/result文本中的旧 marker不使实现 evidence失效；production/source/test reverse
  search出现 marker或 roots 才失效。

## 11. 未决问题

无需要用户决定的问题。

以下不是开放选项，而是由 F402 不变量直接推出的实现约束：

- PackageArtifact v8、Package build identity v9、ServiceDeploymentInput v3；
- Local ABI、ServiceProtocol、最终 ServiceDeployment generation不因本迁移升代；
- deployment input使用 exact `PackageCallableId`；
- revision 对 `serviceCalls` order canonical；
- service-only current sources迁为 Package+Service roots，不恢复 compiler owner。
