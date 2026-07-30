# P5-F415 Collection mapping current-integration completion

状态：Ready。

## 直接父节点

- `P5-F393-collection-mapping-transport-admission-result.md`
- `P5-F409-typed-service-selection-contract-driver-result.md`
- `P5-F411-runtime-router-test-fixture-generation-sync-result.md`

F393 在旧 integration 上形成了 production/tests safe checkpoint，但按 owner 边界刻意缺少
`test-runner::canonical_package_bindings` 的一行 exact copy。F407–F411 随后切换了
PackageArtifact v8/build v9、exact deployment callable 与 test-runner fixtures。本节点从最新
integration 重新移植 F393 语义，不能整分支合并或恢复任何旧 service selection/generation。

## 精确代码状态与候选

- current Skiff production start：
  `0ba321b8870c69e6c737e816ac443fd5e987f0a5`。
- F393 implementation checkpoint：
  `9392f2faf1043e522741602527c271962d498087`。
- F393 result checkpoint：
  `018818878221e0e03935add0e0171bde63bac22c`。

先逐文件比较 checkpoint 与 current tree。可用 `cherry-pick --no-commit` 辅助获取冲突，但只保留
仍符合当前模型的 mapping-owned hunks；不得提交 merge commit。

## 写入范围

允许：

```text
artifact-model/src/{collection_mapping.rs,compile_requirements.rs,deployment.rs,lib.rs}
artifact-identity/src/{deployment/**,package_artifact/**,runtime_assembly/**}
compiler/input-model/src/dependencies.rs
compiler/{driver,emission,projection-input,projection,tests}/**
deployment/**
runtime/{host,linked-program,linker,loader}/**
test-runner/src/package_test_assembly.rs
本任务result
```

只有 checkpoint 中 mapping constructor 的 current-generation 机械跟随才可触及上述目录；不得修改
Router、scripts、其它 test-runner 文件、design、Internals、skiff-packages 或 stable/live。

## 必须实现

1. 保留 F393 的逐跳 exact fact：

```text
package.yml dependency mapping
  -> PackageDependency
  -> PackageRequirement
  -> ServiceDeployment PackageBinding
  -> RuntimeAssembly package link
  -> linker / loader admission
  -> Host DB metadata collection name
```

2. `PackageRequirement.collection_name_mapping` 与
   `PackageBinding.collection_name_mapping` 使用 canonical `BTreeMap`；missing/empty 同一表示，
   key 顺序不影响 identity。
3. mapping 变化继续改变拥有 edge 的 Package build、deployment 与 assembly identity，但不改变
   Package Local ABI；不提升 schema generation，不新增兼容 wire。
4. 所有层 exact 比较，继续拒绝 unknown source、partial/explicit target collision、service own
   collection collision、跨 dependency collision、deployment/assembly drift 与 ambiguous active edge。
5. Host 实际把 `package_secret` 投影到 `mapped_package_secret`，未映射 collection 保持 source name，
   reload/recovery 保持相同 metadata。
6. 在 current `test-runner/src/package_test_assembly.rs::canonical_package_bindings` 增加：

```rust
collection_name_mapping: requirement.collection_name_mapping.clone(),
```

   不得使用空 map 代替 requirement fact。
7. 适配当前模型时：
   - PackageArtifact 保持 v8、build v9、Local ABI v6；
   - 不恢复 `service_call_roots` / `PackageServiceCallRoot`；
   - deployment input 保持 exact `package_callable_id`；
   - RuntimeAssembly 保持 v2 `gateway_ingress`；
   - test-runner 保持 F411 的 test-service/T1/T2 语义。

若 checkpoint 的 mapping 语义要求修改未授权的新 production owner，或一次有界移植后仍有多个不明确
方向，返回 `TASK_SCOPE_EXPANDED`，不要扩大。

## 验证与交付

至少运行并记录实际测试数：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo check --locked \
    -p skiff-artifact-model -p skiff-artifact-identity \
    -p skiff-compiler-input-model -p skiff-compiler \
    -p skiff-deployment -p skiff-runtime-linked-program \
    -p skiff-runtime-loader -p skiff-runtime-linker \
    -p skiff-runtime-host -p skiff-test-runner

CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler \
    --test generated_service_deployment \
    real_package_fixture_transports_collection_mapping_to_runtime_assembly
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-linker --lib assembly
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-loader --lib runtime_assembly
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-host \
    --lib loader::assembly_admission::tests::full_chain -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-test-runner \
    --test package_service_contract_deployment -- --test-threads=1
cargo fmt --all -- --check
git diff --check
```

可补跑 F393 的 identity/model/deployment 聚焦 tests，但不得运行 stable/live 或完整 isolated suite。
不得派子 Agent。

写 `P5-F415-collection-mapping-current-integration-result.md`，记录 checkpoint hunk 映射、冲突适配、
exact commit/tree、测试计数、identity/admission矩阵。提交并保持 clean；不 merge/rebase/push。
