# P5-F445C Package interface identity normalization result

状态：`IMPLEMENTATION_PASS`。没有触发 `TASK_SCOPE_EXPANDED`。

本 leaf 严格沿 P5-F445A 的单一路径完成 source compiler semantic comparison 修复：
`TypeResolutionModel::canonicalize_type_ref` 现在会解析并递归规范化
`AnyInterface.interface_abi_id` 中嵌入的 `TypeRefIr`，再用 canonical identity 与 canonical
type args 重建 interface instantiation id。同一 exact package/symbol/ABI/generic args 的
`Dependency` 与 `PackageId` owner 形式因此相等；不同 package、symbol、ABI、generic args
以及未绑定或 malformed identity 继续 fail closed。

没有修改 dependency-local rehydration、artifact projection/publication、linker、runtime、
artifact/receipt schema 或 Internals package source。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| 声明的 production 输入 | `6e5d77fe` | `7038d29bef3edd8930e0bbe943a43a9d0271b62c` |
| 直接父 result | `5d542728` | `063e8198d9a7ecb810dcf2788dbb87c1294a11a3` |
| task dispatch / worktree 起点 | `d7596b4b` | `dee35d66ac5f907f923b1189305cba06da5a49c6` |
| implementation | `f50c4a774b5e7700bf4652be749151e086ba644a` | `cb793f6df29280a127063e0a333cba7b4ed9a4eb` |

implementation 精确修改：

- `compiler/source/src/type_resolution_model.rs`
- `compiler/tests/package_interface_identity.rs`
- `compiler/Cargo.toml`

除此之外只新增本文 result。

## 2. Test-first RED

production 未修改时，先注册并运行独立 package-project fixture。

首个 direct fixture 结果为 `0 passed, 1 failed`。诊断同时列出 expected 与 found 的内部
identity，二者具有同一：

- package id：`example.com/interface-provider`
- public symbol：`Handler`
- exact ABI：
  `skiff-package-local-abi-v7:sha256:2b6b70c8b858a3ee88df957eb0488a98224fd928669c84021f15aecf7de464e6`
- generic args：空

唯一 owner 差异是：

```text
expected package: Dependency { dependency_ref: "provider" }
found package:    PackageId { package_id: "example.com/interface-provider" }
```

同一次 RED 覆盖 direct parameter、return/local annotation、nullable、array、inline record
三种 field carrier和 generic interface；nested record 诊断进一步直接显示差异存在于序列化的
`InterfaceInstantiationRef.interface_abi_id` 内，而非普通 outer type。

完整矩阵在 production 未修改时结果为：

```text
running 4 tests
1 passed; 3 failed
```

失败项：

1. direct package callable 的 `Dependency` / `PackageId` owner 等价正例；
2. dependency-owned interface 经 facade public signature 暴露的 transitive 正例；
3. 直接构造 exact identity 的双向 canonical assignability 正例。

负向 source matrix 在 RED 阶段已经通过，证明 package/symbol/generic args 的拒绝并不依赖本修复。

## 3. 实现

唯一 production 变化位于
`TypeResolutionModel::canonicalize_type_ref` 的 `TypeRefIr::AnyInterface` arm：

1. 先递归 canonicalize `canonical_type_args`；
2. 将 `interface_abi_id` 解码为 `TypeRefIr`；
3. 对嵌入 identity 递归调用现有 `canonicalize_type_ref`；
4. 调用既有 `interface_instantiation_ref`，以 canonical identity 与 canonical args 重建
   canonical JSON id；
5. 解码失败时保留原 malformed identity，只规范化 args，不猜 package、不去掉 owner、不按 ABI
   字符串放宽相等。

普通 `PackageSymbol` 的既有规则仍是唯一 canonical owner 解析规则：dependency alias 只在有 exact
binding 时映射到 package id，symbol path 与 ABI expectation 均保留。没有新增 cast、source spelling
特例、compatibility adapter、dual path 或 fallback。

## 4. 验收矩阵

新增 test target：`package_interface_identity`，共 4 tests。

| 场景 | 结果 / 证据 |
| --- | --- |
| provider-owned direct parameter | GREEN |
| provider-owned return 再赋给 consumer annotation | GREEN |
| `any Handler?` | GREEN |
| `Array<any Handler>` | GREEN |
| inline record 的 direct / nullable / array fields | GREEN |
| matching generic `any GenericHandler<string>` | GREEN |
| facade public signature 暴露 dependency-owned interface，consumer 直接依赖同一 exact package | GREEN |
| transitive return、nested nullable array 与 generic | GREEN |
| `Dependency` / `PackageId` 双向 canonical comparison | GREEN |
| 不同 package 但伪造同 symbol/ABI 字符串 | RED / 不可 assign |
| 同 package/ABI、不同 symbol | RED / 不可 assign |
| 同 package/symbol、不同 ABI expectation | RED / 不可 assign |
| 同 generic interface、不同 canonical type args | RED / 不可 assign |
| 未绑定 dependency alias | RED / 不可 assign |
| malformed embedded identity 对合法 identity | RED / 不可 assign |
| source 中不同 package、symbol、generic args | compile RED，三项诊断均被断言 |
| 既有 dependency-local nested rehydration baseline | 继续 GREEN |

transitive fixture 还断言 facade 的 provider artifact 保持 canonical
`PackageId(example.com/interface-base)` owner；consumer-local dependency rehydration 没有被逆转或删除。

## 5. Artifact / Local ABI / receipt identity invariance

在 production 修改前，独立 direct provider fixture 的 identity 为：

```text
package build:
skiff-package-build-v10:sha256:3b9f3647318e5da0a7698be305309f5b18f0e0cbfdf256b6fc1fd7d5162116ef

package Local ABI:
skiff-package-local-abi-v7:sha256:2b6b70c8b858a3ee88df957eb0488a98224fd928669c84021f15aecf7de464e6

published receipt identity:
skiff-package-build-v10:sha256:3b9f3647318e5da0a7698be305309f5b18f0e0cbfdf256b6fc1fd7d5162116ef
```

GREEN test 将这些 pre-fix 值固化为精确断言，并额外比较：

- standalone provider 与 consumer graph 中 provider 的完整 `PackageArtifact` 相等；
- `package_build_id` 相等；
- `package_local_abi.local_abi_identity` 相等；
- `package_artifact_ref`（path-free receipt identity）相等。

因此修复没有泄漏到 provider projection/publication，也没有更改 artifact、Local ABI 或 receipt
identity。首次被错误拒绝的 consumer 会产生自己的正常 artifact，这是预期 acceptance 变化。

## 6. GREEN 验证

父 result 的四条命令全部使用任务隔离 target
`/Users/geek/workspace/skiff-p5-f445c-interface-identity/build/cargo-target`：

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-compiler-source package_interface_conformance_stays_owned_by_canonical_package_facts -- --nocapture` | PASS：1 passed、0 failed、322 filtered out |
| `cargo test -p skiff-compiler-source package_signature_exact_symbols_rehydrate_and_ownerless_slots_fail_closed -- --nocapture` | PASS：1 passed、0 failed、322 filtered out |
| `cargo test -p skiff-compiler --test package_imports dependency_callable_local_parameter_preserves_schema_result_field_types -- --exact --nocapture` | PASS：1 passed、0 failed、10 filtered out |
| `cargo test -p skiff-compiler --test package_interface_identity -- --nocapture` | PASS：4 passed、0 failed |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

聚焦测试合计 7 passed、0 failed。Cargo 只报告仓库既有 unused/dead-code warnings。

### 共享 target 环境遮挡

RED 阶段最初按任务声明使用共享 target
`/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target` 并成功得到真实测试失败。
production 修复后的首次重跑被并行 F445D worktree 写入的同名 crate artifact 遮挡：
rustc 明确从
`/Users/geek/workspace/skiff-p5-f445d-timeout-syntax/syntax/src/ast.rs`
读取新增 AST variants，导致本 worktree 的旧 compiler match 出现 21 个非穷尽错误。

该失败发生在 test execution 前，不是本实现 GREEN 失败。父任务确认是并行 worktree 的共享
`CARGO_TARGET_DIR` path 污染，并要求：

- 不清理共享 target；
- 不扩写 compiler matches；
- 后续命令改用本任务隔离 target；
- 在 result 记录遮挡与隔离恢复。

隔离 target 从本 worktree 的 `skiff-syntax` 重新构建后，上述 4 条聚焦命令全部 PASS。

## 7. Scope、反向核对与禁令

- implementation commit 相对 task 起点只含声明的 3 个 implementation 文件。
- production diff 只改 `canonicalize_type_ref` 的 `AnyInterface` arm。
- 没有修改 `canonicalize_type_ref_for_module`、dependency-local rehydration、
  `artifact-model/**`、`compiler/projection/**`、`compiler/driver/**`、
  `compiler/lowering/**`、`runtime/linker/**`、`runtime/linked-program/**`、
  `runtime/linked-type-plan/**` 或任何 Internals 文件。
- 没有启动或修改 stable instance、watch registry、router、runtime、telemetry、MongoDB 或本地
  service。
- 没有运行 live、network、reload 或 fixed-port workload。
- 没有派生子 Agent，没有 merge、rebase 或 push。
- implementation 与 result 分开提交；result commit/tree 由交付消息记录。
