# P5-F415 Collection mapping current-integration result

状态：Complete。

## 1. 提交锚点与范围

| 锚点 | commit | tree |
| --- | --- | --- |
| current production start | `0ba321b8870c69e6c737e816ac443fd5e987f0a5` | `a0c0112bc4285cbbda9e0a8befe3df666d4ddea4` |
| task definition / implementation parent | `91e5475d18af9b30adcc01dc4ea2ba41e3d1e10b` | `186a877f8259f274cd79a7588fc204c1e2bec467` |
| F393 implementation checkpoint | `9392f2faf1043e522741602527c271962d498087` | `7e7c3124c90295cbf719a1bb37a0613da63dfc0c` |
| current implementation end | `25fa06ed5baa8c56d829abae699dbf175146501f` | `02cac60583ef001cb80d5199c5a83c75cfb81b48` |

实现提交：

```text
25fa06ed5baa8c56d829abae699dbf175146501f
runtime: complete dependency collection mappings
```

实现提交修改 33 个文件：F393 checkpoint 的 32 个 mapping-owned 文件，加任务明确要求的
`test-runner/src/package_test_assembly.rs` exact copy。没有修改 Router、scripts、设计文档、
Internals、skiff-packages、stable/live 配置或本地 instance。

## 2. Checkpoint hunk 映射与 current 适配

从 clean `91e5475d` 对 `9392f2fa` 执行 no-commit 三方移植。只有
`artifact-identity/src/package_artifact.rs` 的 test import 集合产生文本冲突；解决方式是在 current
的 typed service selection imports 中加入 checkpoint 需要的 `PackageRequirement`，没有恢复被删除的
类型。

逐文件审计结果：

- `artifact-model/src/collection_mapping.rs` 与 checkpoint 内容 byte-equivalent。
- 除下述 current 适配外，其余 30 个 checkpoint 文件的 add/delete line delta 与原提交逐文件一致。
- `artifact-identity/src/package_artifact.rs` 只因 current import 集合多一行格式差异；mapping identity
  test 语义与 checkpoint 相同。
- `compiler/tests/generated_service_deployment.rs` 保留 checkpoint 的 fresh dependency、PackageArtifact
  requirement、deployment binding 与 assembly link 三跳断言，但将旧
  `api.yml { source, serviceCall }` 改为 scalar `read: main.read`，并在
  `service.yml.serviceCalls` 选择 `read`；显式 `ServiceManifestAuthoring` 同步同一 selection。
- `test-runner/src/package_test_assembly.rs::canonical_package_bindings` 新增：

  ```rust
  collection_name_mapping: requirement.collection_name_mapping.clone(),
  ```

  没有用 empty map、推断或 fallback 代替 requirement fact。

最终 added-line 反向搜索没有引入 `PackageServiceCallRoot`、`service_call_roots`、
`package_public_path`、`global_ingress`、v7 PackageArtifact 或 v8 Package build 标识。
current constants 仍为：

```text
PackageArtifact schema  skiff-package-artifact-v8
Package build           skiff-package-build-v9:sha256
Package Local ABI       skiff-package-local-abi-v6:sha256
RuntimeAssembly schema  skiff-runtime-assembly-v2
```

generated deployment 继续写 exact `package_callable_id`；RuntimeAssembly 与 test-runner 继续使用
`gateway_ingress` 以及 F411 的 test-service/T1/T2 路径。

## 3. 逐跳 exact fact

最终事实流为：

```text
package.yml packages[].collection_name_mapping
  -> PackageDependency.collection_name_mapping
  -> PackageRequirement.collection_name_mapping
  -> ServiceDeployment PackageBinding.collection_name_mapping
  -> RuntimeAssembly package link collection_name_mapping
  -> linked image / linker / loader exact edge admission
  -> Host DbMetadataIr.collection_name
```

`PackageRequirement` 与 `PackageBinding` 都使用 `BTreeMap<String, String>`。DTO 使用
`serde(default, skip_serializing_if = "BTreeMap::is_empty")`，因此 missing 与 empty 反序列化为同一值，
canonical wire 只保留一个 empty 表示，插入顺序不进入 identity。

compiler requirement producer、generated deployment producer与 test-runner canonical fixture producer
都从上一跳 exact clone；deployment resolver 保留完整 `PackageBinding` 进入 assembly link plan。
linked image、linker与 loader 都比较 requirement、deployment binding 与 canonical assembly link 的
完整 mapping。

## 4. Identity 矩阵

| 变化 | Package build | Package Local ABI | deployment | assembly |
| --- | --- | --- | --- | --- |
| missing vs empty | 相同 | 相同 | 相同 | 相同 |
| map key 插入顺序变化 | 相同 | 相同 | 相同 | 相同 |
| empty -> single mapping | 变化 | 不变 | 变化 | 变化 |
| 同 source 的 target 变化 | 变化 | 不变 | 变化 | 变化 |
| single -> multi mapping | 变化 | 不变 | 变化 | 变化 |

Package build projection包含 canonical package requirements；Local ABI projection仍只包含 package id 与
public symbols。deployment identity包含 package bindings，assembly identity包含完整 package link plan。
没有提升任何 schema generation，也没有兼容 wire、dual-read 或 dual-write。

## 5. Admission 与 Host projection 矩阵

| 场景 | owner / gate | 结果 |
| --- | --- | --- |
| empty source、empty target、两个显式 source 同 target | shared mapping validation + artifact identity admission | reject |
| mapping 引用 dependency 未声明 source | hydrated loader exact source set | reject |
| 显式 target 与未映射 source partial collision | source-to-target projection | reject |
| requirement / deployment binding mapping drift | projection、assembly resolver、linked image | reject |
| deployment binding / assembly link mapping drift | linker、loader canonical link comparison | reject |
| dependency target 与 service own collection collision | per-activation loader collection set | reject |
| 两个 dependency target collision | per-activation loader collection set | reject |
| 同一 DB-bearing package build 通过多个 active edge 投影 | per-activation projected build ownership | reject ambiguous edge |
| mapped + unmapped dependency collections | Host activation metadata | mapped source 使用 target；未映射 source 保持原名 |
| committed recovery / reload | Host context rebuild | 两次 provider metadata 完全相同 |

Host 交给 DB provider 的实际 metadata 将 `package_secret` 投影为
`mapped_package_secret`，同时保留未映射 `package_audit`。测试使用捕获型 provider 与
`mongodb://fixture.invalid`，没有建立 MongoDB 连接。

## 6. 验证证据与实际计数

required Rust 命令均使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 实际结果 |
| --- | --- |
| required 10-package `cargo check --locked` | PASS；0 tests；只有既有 warnings |
| compiler `generated_service_deployment::real_package_fixture_transports_collection_mapping_to_runtime_assembly` | 1 passed / 0 failed；11 filtered |
| `skiff-runtime-linker --lib assembly` | 30 passed / 0 failed；21 filtered |
| `skiff-runtime-loader --lib runtime_assembly` | 17 passed / 0 failed |
| Host `full_chain` | 7 passed / 0 failed；261 filtered |
| test-runner `package_service_contract_deployment` | 23 passed / 0 failed / 1 ignored |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

补充 mapping-owned tests：

| 命令 | 实际结果 |
| --- | --- |
| `skiff-artifact-model collection_mapping` | 2 passed / 0 failed；166 filtered |
| `skiff-artifact-identity collection_mapping` | 1 passed / 0 failed；其余 targets 0 selected |
| `skiff-deployment collection_mapping` | 3 passed / 0 failed；57 filtered |
| compiler lib `authored_dependency_collection_mapping_reaches_compile_requirement_exactly` | 1 passed / 0 failed；26 filtered |

required shared-target commands先在最终实现内容上通过。随后补跑 identity selector 时，共享 target 被另一个
并发 worktree 的旧 `skiff-artifact-model` build 覆盖；两次 shared-target 尝试都在 test execution 前
以“mapping field/function 不存在”结束。没有清理或修改共享 cache，也没有干预其它进程；改用本 worktree
ignored `build/cargo-target` 后同一 identity selector稳定 1/1，通过并确认是共享 build artifact 串扰，
不是本 tree 源码失败。deployment 与 compiler 补充 selector也在该隔离 target通过。

## 7. 自验收

| 条款 | 代码 / 测试证据 | 结论 |
| --- | --- | --- |
| package.yml 到 Host 逐跳 exact fact | compiler fresh fixture + resolver/linker/loader/Host链路 | PASS |
| canonical BTreeMap、missing/empty、order | DTO serde + model/identity/deployment tests | PASS |
| build/deployment/assembly identity变化，Local ABI不变 | identity矩阵与 focused tests | PASS |
| unknown、partial、own/cross-dependency collision | shared resolver + Host 7/7 | PASS |
| drift 与 ambiguous active edge fail closed | linked image/linker/loader exact gates | PASS |
| Host实际 projection 与 reload一致 | captured provider metadata两次相等 | PASS |
| test-runner exact copy | canonical constructor source + 23 pass / 1 ignored | PASS |
| v8/v9/v6、exact callable、v2 gateway、T1/T2保留 | current constants、added-line反向搜索、required gates | PASS |
| ownership与禁止项 | 33 files全部在授权范围；无stable/live/merge/rebase/push | PASS |

结论：F393 的 collection mapping owned 语义已在 current integration 闭合，F393 当时刻意留下的
`canonical_package_bindings` 缺口已用 requirement exact fact 补齐。
