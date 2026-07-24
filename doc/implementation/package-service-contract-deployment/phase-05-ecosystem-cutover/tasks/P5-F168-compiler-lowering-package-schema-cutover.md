# P5-F168：Compiler Lowering Package Schema Cutover

状态：Ready

## 直接父任务

- `P5-F164-compiler-package-schema-import-result.md`

## 当前断点

`PackageTypeRef`已经删除service-owned的`Contract`分支，外部命名类型现在使用声明Package拥有的
schema identity。`compiler/lowering`仍匹配`PackageTypeRef::Contract`，并且接口执行测试仍自行构造
`ServiceContract.boundary_schema`，因此workspace无法继续编译。

## 范围

只修改`compiler/lowering`及其聚焦测试，并写本任务result。不得修改artifact model、compiler
input/source、runtime、service或package。

## 必须实现

- executable type projection消费当前`PackageTypeRef`模型；Package schema命名类型在File IR执行
  类型投影中继续只成为opaque `unknown`，不得把Package ABI identity泄漏或复制到File IR。
- 将接口执行fixture改为真实的Package-owned schema输入：
  - schema type由声明Package拥有；
  - service contract只引用该Package type；
  - compiler dependency中Package引用和service引用绑定到同一个精确Package identity；
  - 不得重建service-owned schema或旧`ContractTypeId`。
- 保持嵌套container/nullable投影、接口方法lowering和File IR不携带边界ABI描述的既有语义。
- 删除本crate内所有旧`PackageTypeRef::Contract`、`ContractTypeId`和
  `ServiceContract.boundary_schema`使用。

## 验证

- `cargo test -p skiff-compiler-lowering`；
- `rg "PackageTypeRef::Contract|ContractTypeId|ContractSchemaType" compiler/lowering`无命中；
- `cargo check --workspace`的首错越过`compiler/lowering`；
- `git diff --check`；
- 独立提交并写`P5-F168-compiler-lowering-package-schema-cutover-result.md`。
