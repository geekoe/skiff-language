# P5-F177：Builtin Artifact 与 Native Type Removal Result

状态：Completed

## 直接父任务

- `P5-F177-builtin-artifact-and-native-type-removal.md`

## 交付

- syntax AST删除`TypeDecl.is_native`，parser删除`native type`声明入口；`native function`及native
  impl method语法保持不变。
- `TypeRefIr::Native`硬切为`TypeRefIr::Builtin`，构造helper同步改为`builtin`；canonical wire只有
  `kind: "builtin"`，`kind: "native"`严格拒绝。
- 删除`TypeDescriptorIr::Native`及`kind: "external"`descriptor wire。
- `NativeTypeExprDef`硬切为`NativeSignatureTypeExpr`，保留TypeParam、Builtin、Array、Map、
  Nullable和Stream，删除ActorRef分支。
- 删除公共native signature registry中的ActorRef常量以及`std.actor.put/get/find/remove`注册项；
  Runtime内部ActorRef DTO未修改。
- 对直接consumer完成`TypeRefIr::Builtin`与`NativeSignatureTypeExpr`机械改名，并删除两个对已移除
  ActorRef签名表达式的穷举分支；未实现actor source typing、std actor API或Runtime actor行为。
- canonical artifact wire在本切点前已经写出`kind: "builtin"`，因此identity golden重算结果不变；
  新增严格拒绝测试证明不存在legacy双wire。

## 验证

通过：

```text
cargo test -p skiff-syntax -p skiff-artifact-model -p skiff-artifact-identity
# syntax: 106 passed
# artifact-model: 110 passed
# artifact-identity: 73 passed
# artifact-identity CLI: 8 passed

rg "TypeRefIr::Native|TypeDescriptorIr::Native|NativeTypeExprDef" \
  syntax artifact-model artifact-identity
# no matches

git diff --check
```

`cargo check --workspace`已成功编译`skiff-syntax`、`skiff-artifact-model`和
`skiff-artifact-identity`，首错位于后续`skiff-compiler-core`的
`TypeDescriptorIr::Native` consumer。该断面属于H34顺序中的compiler source/lowering阶段，本任务
按范围未继续实现。
