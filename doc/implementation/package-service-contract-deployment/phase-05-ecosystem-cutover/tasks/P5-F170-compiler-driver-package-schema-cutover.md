# P5-F170：Compiler Driver Package Schema Cutover

状态：Ready

## 直接父任务

- `P5-F168-compiler-lowering-package-schema-cutover-result.md`

## 当前断点

compiler driver仍导入已删除的service-owned type helper，重建contract dependency时未传入已解析的
Package schema，并且deployment projection未收到Package schema record集合。

## 范围

只修改`compiler/driver`及其聚焦测试，并写result。不得修改input/source/lowering、deployment、
artifact model或runtime。

## 必须实现

- 删除`definition_contract_type_id`、`definition_contract_type_ref`旧入口，调用当前Package-owned
  schema投影接口。
- canonical dependency重建时传递同一批精确`ResolvedPackageSchema`，不得从ServiceContract推导、
  复制或合成类型。
- generated deployment把当前编译已解析的Package schema records传给deployment projection。
- Package alias与service alias继续绑定同一精确Package identity；缺record、版本/build/ABI错配
  fail closed。
- 不得加入旧模型兼容分支。

## 验证

- `cargo test -p skiff-compiler`；
- `rg "definition_contract_type_id|definition_contract_type_ref|boundary_schema" compiler/driver`
  无生产代码命中；
- `cargo check --workspace`首错越过compiler driver；
- `git diff --check`；
- 独立提交并写`P5-F170-compiler-driver-package-schema-cutover-result.md`。
