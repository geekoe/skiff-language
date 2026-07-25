# P5-F285 Dependency result field-read fix result

状态：`PASS`；dependency callable 的 owner-local 参数不再导致精确返回类型丢失。

## Exact candidate

- implementation commit：
  `46371e578107fd714a425c76da6c3b97bf7d8259`
- integration merge commit：
  `1d93dbbd176c5c5ff03037dccbe09526ded3840c`
- 直接父结果：
  `P5-F283-dependency-result-field-read-audit-result.md`

## 修复事实

1. callable signature 中的 `LocalType`、`PublicationType`、`ServiceSymbol`、`PackageSymbol`
   以及容器内嵌引用，按 dependency owner 与 artifact slot 递归恢复为唯一 owner-bound 类型。
2. `PackageSchema` 返回类型的 package identity、stable key 和 type id 保持不变；
   普通类型与 exact projection 同时保留。
3. 参数解析失败会产生 owner/slot 诊断并拒绝调用，不再静默丢弃已解析的返回事实。
4. arity、普通 assignability、精确 projection 参数校验继续生效；没有恢复按短名、display、
   shape 或 callable 名称猜测的 fallback。
5. 修改只落在 `compiler/source` 与 `compiler/tests/package_imports.rs`，Agent、Agine、
   artifact identity、runtime 和 F281 error model 均未修改。

## 验证

实现分支实际通过：

```text
cargo test -p skiff-compiler-source \
  package_signature_local_slots_rehydrate_to_dependency_owner
  1 passed

cargo test -p skiff-compiler --test package_imports \
  dependency_callable_local_parameter_preserves_schema_result_field_types -- --exact
  1 passed

cargo test -p skiff-compiler --test package_imports
  11 passed

git diff --check
  PASS
```

该分支刻意基于 A1 strict-model 合入前的可编译基线开发。当前 integration 已合入 F281/A1，
因此完整 compiler 与真实 Agine 链路要等后续 open-error language/artifact consumer 恢复编译后再验收；
不得以此结果宣称生态链路已经重跑通过。

## Handoff

后续 language consumer 必须保留本结果的 owner-aware signature rehydration，不能因适配新的
declaration、throw/catch site 或 open error channel 而恢复“参数含 local slot 就丢弃整个 signature”
的旧 gate。F269 只有在这些 consumer 合流且 compiler 可编译后才重跑 fresh Agine。
