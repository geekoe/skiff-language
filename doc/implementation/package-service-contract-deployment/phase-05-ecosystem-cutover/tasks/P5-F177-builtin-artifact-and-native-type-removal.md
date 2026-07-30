# P5-F177：Builtin Artifact 与 Native Type Removal

状态：Ready

## 直接父任务

- `P5-H34-actor-type-and-builtin-ir-cutover.md`

## 范围

修改`syntax`、`artifact-model`、`artifact-identity`及其直接严格wire/tests。允许对只为这些公共
模型编译而必须机械更新的直接consumer做最小重命名，但不得实现actor source typing、std actor
API或Runtime actor行为。

## 必须实现

- 删除`native type`词法/语法/AST声明与parse/serialize测试；`native function`语法保持。
- 删除`TypeDescriptorIr::Native`。
- `TypeRefIr::Native`重命名为`Builtin`，wire kind从`native`硬切为`builtin`；Skiff未发布，不保留
  legacy deserialize兼容。
- `NativeTypeExprDef`重命名为`NativeSignatureTypeExpr`，保留builtin、Array、Map、Nullable、
  Stream、TypeParam表达力，删除`ActorRef`分支。
- 删除公共模型中source-level ActorRef签名常量/注册项；Runtime内部ActorRef DTO不在本任务删除。
- identity/golden按新canonical wire重算，并证明不存在双wire。

## 验证

- syntax与artifact-model/identity全测试；
- legacy `native type`、type-ref `kind:native`、native descriptor严格拒绝；
- `rg "TypeRefIr::Native|TypeDescriptorIr::Native|NativeTypeExprDef" syntax artifact-model
  artifact-identity`无命中；
- `cargo check --workspace`首错越过公共模型并准确落在后续compiler/std/runtime consumer；
- `git diff --check`；
- 独立提交并写result。
