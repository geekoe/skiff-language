# P5-F14：Linked File Ref Semantics Repair

## 输入、owner与限制

- 输入：D15完成；exact integration `02b97ffe1c3cb4232894dc23d16ad3d61f2c2ba6` / tree
  `1bc4c9d9d6bbae171cc6c6fa7c838c6e6b7f8f8f`，已包含F12/F13 combined checkpoint与R13 PASS。
- 独立worktree/branch，一个clean commit，不merge/push。
- 唯一owner限`runtime/linked-program/src/shared_image.rs`与直接tests；最多允许runtime linker现有assembly test接入一项
  production正例。
- 不改compiler authoring、artifact model/identity、loader/Host admission、test-runner/canonical store/fixture、Router、
  manifest/Cargo.lock、F05或stable。

## 完成态

抽出唯一semantic FileIR target matcher，供callable target validation与`executable_addr`复用：

- file identity exact；
- module path exact；
- nested target的sourceAstHash若存在则必须exact；
- `artifactPath`只作loaded-record locator，不参与semantic target equality；
- file/callable/executable index与target-specific diagnostics保持fail closed。

不得把target None hash视为任意identity/module，不得清空或重写落盘record，不得把path差异扩大为所有ref字段忽略。

## 验证

```bash
cargo test --locked -p skiff-runtime-linked-program shared_image
cargo test --locked -p skiff-runtime-linker assembly
node scripts/check-runtime-crate-dag.mjs
cargo fmt --all -- --check
git diff --check
```

正例必须覆盖storageful top-level + pathless nested callable、随后`executable_addr`及production
`link_runtime_assembly`完成execution image。负例覆盖wrong file identity/module/present source hash、callable/executable
index越界。每个filter非零。回报matcher矩阵、source/commit/tree、single clean/lock、scope/reverse与extra-review；不在
本任务运行或宣称F04最终gate。

## R14 acceptance record

首个candidate因修改linker test fixture被R14判FAIL；F14A从same base重建single candidate
`629f1c815f16c366c67557dfaba01a09455207fd` / tree `5401add98ed9513fd495dd3eba4ac92e7ef3bce2`，fixture
zero-diff且production/shared-image语义不变，独立R14窄复验PASS并合流为`bcbdc2c`。linked-program 11/11、linker
assembly 13/13、DAG与changed-file fmt通过；lock未变。
