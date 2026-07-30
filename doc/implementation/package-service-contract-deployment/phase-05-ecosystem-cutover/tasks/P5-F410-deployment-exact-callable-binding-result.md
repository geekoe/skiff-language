# P5-F410 Deployment exact callable binding result

状态：Complete。

## 1. 提交锚点与范围

| 锚点 | commit | tree |
| --- | --- | --- |
| 任务规定 start | `288a105fc87399c5e93228ee9f2ba2e58c4cd2b6` | `4688200acf69afe8778b06189c545e06d49d7212` |
| task definition checkpoint | `97f3f831b02507a8caa1f831c590ea044655f895` | `9a208b775f009a9795aaaf7714dae16e1f2d3d25` |
| implementation end candidate | `ec98df8eeca10f1b61b10bb32d0b34364be39323` | `95ede584a35912b733baa12fd323adda0d92a5d6` |

实现提交：

```text
ec98df8eeca10f1b61b10bb32d0b34364be39323
feat(deployment): bind operations by exact callable id
```

production/test改动只落在任务授权的`deployment/**`；本文是唯一额外result文件。没有修改
artifact model/identity、compiler、runtime、router、test-runner、ecosystem source或canonical
design，也没有merge、rebase或push。

## 2. Exact callable binding

`project_operation_bindings`现在直接读取每个
`ServiceDeploymentOperationInput.package_callable_id`，不再按public path查找：

- exact ID必须出现在implementation PackageArtifact的Package Local ABI
  `public_symbols` callable中；
- `implementation_symbols`或仅存在于`callableLinks`的非public ID明确返回
  `NonPublicPackageCallable`；
- callable link的map key、nested `callableId`、target `callableAbiId`必须是同一exact ID；
- link target只有两种合法分类：非instance成员的`PublicFunction`，或列入
  `PublicInstance.methods`的`ImplMethod`；
- boundary projection、callable semantic facts、eligibility和implementation requirements全部按
  输入的同一exact ID读取；
- final `DeploymentOperationBinding.package_callable_id`原样保存输入exact ID。

完整operation set规则保持：重复operation、未知/额外operation和遗漏operation全部拒绝。Available
boundary的descriptor仍与contract operation descriptor精确比较；Unavailable、facts/requirements不一致
和link forgery继续fail closed。PackageArtifact在进入operation projection前先通过canonical identity/surface
validation，因此篡改link target会在admission最前端以`InvalidTypedArtifact`拒绝；operation projection也保留
同ID/link kind的逐项检查。

## 3. Fixture与generation

- deployment内3个`ServiceDeploymentOperationInput` literal全部改为v3
  `package_callable_id`。
- deployment内5个PackageArtifact fixture literal（分布在4个文件）删除已不存在的
  `service_call_roots`，由共享常量生成v8 artifact和v9 build identity。
- ServiceDeployment output仍为`skiff-service-deployment-v2`。
- deployment identity仍使用`skiff-deployment-artifact-v2:sha256`。
- 测试同时断言input v3、output v2、identity v2 prefix、wire包含`packageCallableId`且不包含
  `packagePublicPath`。

## 4. Exact-ID验证矩阵

| 场景 | 代码/测试证据 | 结果 |
| --- | --- | --- |
| exact public function | `projection_maps_every_operation_explicitly_and_emits_no_public_path` | exact ID原样进入两个final bindings |
| exact public-instance method | `exact_public_instance_method_is_admitted_and_preserved`构造完整有效Local ABI/receiver/interface/link surface | PASS |
| missing wire member | 从v3 input wire删除`packageCallableId` | strict deserialize拒绝 |
| empty/forged/foreign ID | `missing_forged_and_implementation_only_callable_ids_fail_closed` | `UnknownPackageCallable` |
| implementation-only/private ID | 同一测试把有效callable迁到`implementation_symbols`并改为`InternalFunction` | `NonPublicPackageCallable` |
| wrong/duplicate/missing/extra contract operation | `operation_mapping_failures_are_structured_and_fail_closed` | 对应structured error |
| Unavailable | `unavailable_callable_and_nominal_descriptor_mismatch_fail_closed` | `BoundaryUnavailable` |
| descriptor/Package-owned type mismatch | 上述测试及`package_owned_operation_requires_exact_owner_key_and_type_id` | `OperationContractMismatch` |
| facts与requirements mismatch | `callable_facts_requirements_and_link_target_mismatches_fail_closed` | `CallableFactsMismatch` |
| forged link target | 同一测试篡改target `callableAbiId` | canonical PackageArtifact admission拒绝 |
| operation set completeness | missing、duplicate和unknown/extra正反例 | fail closed |
| output/generation/identity | projection positive test的v3/v2/prefix/wire断言 | 保持ServiceDeployment v2 |

## 5. 反向搜索

```text
rg '\bpackage_public_path\b|\bservice_call_roots\b|\bPackageServiceCallRoot\b' deployment
=> 0 files

rg 'service\.ya?ml|ServiceManifestAuthoring' deployment
=> 0 files

rg 'packagePublicPath' deployment
=> deployment/src/projection/tests.rs:347
```

最后一处只是否定断言，确认canonical output wire不含旧字段；不存在reader、lookup、fallback或
selection rebuild。

```text
rg 'ServiceDeploymentOperationInput\s*\{' deployment
=> 3 literals，全部使用package_callable_id
```

## 6. Test discovery与执行

按任务要求，先对两个selector执行`-- --list`，再运行同一selector：

| selector | `-- --list`实际选择 | 执行 |
| --- | ---: | --- |
| `skiff-deployment projection` | 19 | 19 passed |
| `skiff-deployment storage` | 13 | 13 passed |

执行命令：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-deployment projection -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-deployment storage -- --list

CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-deployment projection
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-deployment storage
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo check --locked -p skiff-deployment
cargo fmt --all -- --check
git diff --check
```

结果：

- projection：`19 passed / 0 failed`；
- storage：`13 passed / 0 failed`；
- cargo check：PASS；
- fmt：PASS，无输出；
- diff check：PASS，无输出。

没有运行workspace/isolated/stable/live、instance、生态publish或外部服务，也没有派子Agent。

## 7. 自验收结论

P5-F410已完成deployment consumer的exact callable cutover：输入不再携带或解析public path，admission只接受
implementation artifact中的public function/public-instance method exact ID，全部既有boundary、descriptor、
facts、requirements、operation-set和identity检查保持fail closed；ServiceDeployment generation与identity
保持v2。该提交只解除任务声明的deployment consumer节点，不宣称F409 producer或整个阶段已完成。
