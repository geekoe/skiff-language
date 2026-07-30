# P5-F06：Callable Effects Exact Dependency Transfer

## 输入、owner与限制

- 依赖：P5-D07完整审计；使用独立worktree/branch，只提交一个clean commit，不merge/push。
- 唯一production owner是`compiler/source callable-effects`：
  `transfer/{call,expression,statement}.rs`、必要的`dependency_analysis.rs` exact lookup，以及source/direct compiler
  integration tests。
- 不改projection eligibility、service-call lowering、compiler artifact/wire、runtime/Host/Router/test-runner或
  F04 fixture；不增加宽泛heap模型、cast、compat shim、artifact patch/re-sign。root lock不得改。

## 冻结transfer语义

1. exact `DependencyPackageFunction` / `ContractOperation`作为call-position callee时不先触发standalone
   dependency-address unknown；表达式求值顺序保持。未解析、动态使用、first-class value或非call位置仍fail closed。
2. 只有exact descriptor、unary、无callback、无typed error且六项detached guarantee全真的contract operation生成
   known callee state：mutation/alias/escape/same-heap/unknown均false，`may_suspend`取descriptor，return provenance为
   Fresh。其它descriptor与missing/ambiguous alias/operation保持unknown。
3. 只精确建模直接标量参数字段写（例如`input.value = "helper-mutated"`）：helper facts仅
   `writes_caller_reachable=true`且provenance保持Analyzed。nested/index/reference-valued/unknown RHS仍fail closed。
4. 复用既有`apply_callee`按actual provenance传播：helper自身boundary必须Unavailable(WritesCallerReachable)；
   consumer把fresh record传给helper时不产生caller-reachable write，随后全detached contract call使consumer wrapper
   Available。不得硬编码fixture symbol/package/service。

## 正负证据

正常source integration必须构造：helper `Box` + `mutate`、consumer fresh Box → package direct mutate →
`payments/echo(box.value)`；provider按输入分支返回detached常量。最终compile projection中helper mutate保持
Unavailable，consumer run为Available；缺helper mutation、wrong binding或non-detached contract均失败。

至少增加并非零运行：

- `exact_dependency_callee_does_not_poison_known_target`
- `detached_contract_target_uses_descriptor_effect_guarantees`
- `non_detached_or_unsupported_contract_remains_fail_closed`
- `direct_scalar_parameter_field_store_has_only_write_effect`
- `nested_or_reference_heap_store_remains_fail_closed`
- `package_direct_mutation_then_detached_contract_projects_available`

## 聚焦验证

```bash
cargo test -p skiff-compiler-source exact_dependency_callee_does_not_poison_known_target
cargo test -p skiff-compiler-source detached_contract_target_uses_descriptor_effect_guarantees
cargo test -p skiff-compiler-source non_detached_or_unsupported_contract_remains_fail_closed
cargo test -p skiff-compiler-source direct_scalar_parameter_field_store_has_only_write_effect
cargo test -p skiff-compiler-source nested_or_reference_heap_store_remains_fail_closed
cargo test -p skiff-compiler --test service_conformance package_direct_mutation_then_detached_contract_projects_available
cargo test -p skiff-compiler-lowering one_contract_call_uses_the_typed_requirement_and_operation_identity
git diff --check
```

回报每个callee/write分类的facts、helper/consumer projection、exact source/commit/tree、single commit/clean/lock与
reverse-search。不得运行full compiler gate；合流及R06 PASS后F04才恢复真实isolated positive。
