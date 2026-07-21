# P5-F06A：Exact Callee Field Repair

## 输入、owner与限制

- 输入：R06在`2982cd8d5182384d8debb2a5fa55dbfe4f5e979a` / tree
  `642892b6e03b86ed737cde20462411c4769df043`的首次FAIL；使用独立worktree/branch，只提交一个clean commit，
  不merge/push。
- 唯一production owner是`compiler/source/src/callable_effects/transfer/call.rs`；只允许增加直接source/compiler
  integration回归测试。不得修改parser、resolver、dependency descriptor、store transfer、projection、lowering、
  artifact、runtime、router、test-runner、manifest或root lock。
- 不按alias/public path/symbol写fixture分支，不放宽standalone、first-class、dynamic或unresolved dependency address。

## 冻结修复语义

1. 只有call key已解析为exact `DependencyPackageFunction`或`ContractOperation`时，exact callee求值才可沿
   `Generic`和`Field` wrapper递归到其`DependencySourceAddress` object，并在保持preorder expression key/index
   一致的前提下跳过standalone address unknown污染。
2. canonical `alias/public.method()`必须覆盖真实AST
   `Field(DependencySourceAddress(alias/public), method)`；同一语义覆盖stable key含成员路径的全detached contract
   operation。
3. 非exact target、field作为first-class value、普通receiver field、dynamic/unresolved call仍走既有`eval_expr`
   fail closed；不得把任意Field receiver当作安全常量。
4. F06已关闭的detached descriptor与direct scalar store逻辑不得改动；差异相对F06 merge第一父只包含callee
   wrapper修复与直接测试。

## 正负证据与gate

至少新增一个使用canonical source拼写`alias/public.method()`的package exact正例，并证明effects不含
same-heap/unknown-target/suspend且projection Available；增加field first-class或unresolved负例证明仍fail closed。
如contract fixture具备同形态，必须同时覆盖全detached contract exact正例。

```bash
cargo test -p skiff-compiler-source exact_dependency_field_callee_does_not_poison_known_target
cargo test -p skiff-compiler-source exact_dependency_callee_does_not_poison_known_target
cargo test -p skiff-compiler-source detached_contract_target_uses_descriptor_effect_guarantees
cargo test -p skiff-compiler-source non_detached_or_unsupported_contract_remains_fail_closed
cargo test -p skiff-compiler-source direct_scalar_parameter_field_store_has_only_write_effect
cargo test -p skiff-compiler-source nested_or_reference_heap_store_remains_fail_closed
cargo test -p skiff-compiler --test service_conformance package_direct_mutation_then_detached_contract_projects_available
cargo test -p skiff-compiler-lowering one_contract_call_uses_the_typed_requirement_and_operation_identity
git diff --check
```

每个filter必须非零。回报canonical AST形态、exact与非exact facts差异、source/commit/tree、single commit/clean/lock
及reverse-search。合流和组合probe后，原R06 reviewer只窄复验首次FAIL项；R06 PASS前F07与F04仍锁定。
