# P5-F162：Compiler Package Schema Input Result

状态：Completed

## 直接父任务

- `P5-F162-compiler-package-schema-input.md`

## 交付

- `skiff-compiler-projection-input`新增只读`ResolvedPackageSchema`：
  - 绑定exact dependency alias、Package id、version、build与local ABI；
  - 携带已由canonical store验证的`PackageSchemaIndex`和逐类型records；
  - 字段私有，只通过只读accessor和`public_type`查询；
  - 构造时拒绝owner、index/record闭合或key/id不一致。
- public-only决策在compiler input边界fail closed：
  - index entry必须是`PublicNameable`；
  - `publicPath`必须存在并等于作为`stableSchemaKey`的canonical public API path；
  - `ClosureOnly`保留为未来wire枚举，但当前input拒绝。
- `PackageCompileInput`现在可接收：
  - 测试/嵌入方已解析的schema DTO；
  - authoring driver持有的只读`CanonicalArtifactStore` resolver。
- driver在File IR补齐实际Package requirement closure之后才解析schema：
  - 普通显式dependency和隐式exact `skiff.run/std`走同一个选择与store解析函数；
  - 未实际引用std时不会提前读取其schema；
  - 缺schema、重复binding、错owner、错version/build/local ABI或artifact identity均以
    `PackageSchemaInput`结构化错误拒绝。
- `PackageArtifactProjectionInput`取得已经裁剪为实际requirements的schema DTO slice；projection crate没有
  filesystem/store依赖。
- 清理了projection-input测试中残留的`ContractTypeId`/`PackageTypeRef::Contract`测试构造，没有恢复旧模型。

## std与当前Package断面

本节点建立了official std与普通Package完全相同的consumer解析通道：std只有在File IR产生隐式exact
requirement后，才通过其canonical PackageArtifact调用
`CanonicalArtifactStore::resolve_package_artifact_schema`，consumer端没有
`canonical_http_boundary_type`或第二份descriptor生产逻辑。

std自身schema records的生成，以及当前Package刚投影出的records交给ServiceContract closure，仍属于直接父
结果所阻塞的F161 Package schema projection。F159硬切后该projection尚未生成任何Package的index/records；
本任务没有用空index、HTTP结构特判或compat字段伪造它们。现在F161可以直接消费本节点提供的input API恢复：

1. 当前Package projection生成owned records；
2. official std作为普通Package生成HTTP公开类型records；
3. service projection接收刚生成的同一records view；
4. dependency/std named type从本节点resolved inputs读取。

## 验证

通过：

```text
cargo test -p skiff-compiler-projection-input --lib
6 passed; 0 failed

cargo check -p skiff-compiler-projection-input --lib
passed

git diff --check
passed
```

聚焦测试覆盖：

- exact public schema只读查询；
- 未公开/`ClosureOnly` named type拒绝；
- requirement local ABI不一致拒绝；
- PackageArtifact build binding不一致拒绝；
- nested callable signature使用`PackageSchemaTypeId`，不再构造旧contract-owned type。

完整driver crate仍在编译本任务代码前命中F159 result已记录的旧下游断面：

```text
cargo check -p skiff-compiler --lib
compiler-contract/compiler-input: 14处ContractTypeId、boundarySchema、
Contract/PackagePublic旧模型错误
compiler-projection: 3处旧Contract/PackagePublic错误，以及尚未由F161填充的
PackageArtifact packageSchemaIndex/packageSchemaTypeRecords
```

错误列表没有新增本任务文件的诊断。按任务要求未增加兼容层；F161恢复后必须补跑store-backed真实driver、
official std bootstrap与完整compiler聚焦测试。
