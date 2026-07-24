# P5-F164：Package Schema Consumer Import Result

状态：Completed

## 直接父任务

- `P5-F164-package-schema-consumer-import.md`

## 交付

- `ResolvedContractDependency`现在必须同时接收已验证的`ResolvedPackageSchema`：
  - 公开性来自Package schema index中的`publicPath + PublicNameable`证据；
  - 按ServiceContract的`PackageTypeRequirement`抽取精确type record集合；
  - 校验record identity、owner、stable key、完整闭包和v1无环规则；
  - operation参数、返回值、typed error、server stream与callback interface引用逐项校验；
  - requirement/record集合必须与operation可达闭包精确相等，缺失、额外、未require或不可达record均fail closed。
- `ContractDependencyIndex`删除`ContractTypeId`与`boundarySchema`索引，改为Package-owned
  `PackageSchemaTypeRecord`索引。
- source类型解析与materialization改为保留
  `packageId + stableSchemaKey + PackageSchemaTypeId`：
  - `serviceAlias.Type`只负责选择已验证记录，IR使用owner PackageSymbol而非service-owned ServiceSymbol；
  - contract operation的普通参数/返回、stream item和callback descriptor均从Package record读取；
  - assignability比较完整Package三元身份，不按display string或结构相同放宽；
  - direct package alias和service alias可解析到完全相同的Package record。
- compiler driver把F162 resolved package schema records加入普通Package dependency analysis facts，避免
  direct package import退回本地/display身份。
- 退役只覆盖旧service-owned `ContractTypeId`/closure-only模型的测试fixture，补入Package owner三元身份、
  精确闭包、service alias重命名无关性和真实canonical artifact store round-trip覆盖。

## 验证

通过：

```text
cargo test --offline -p skiff-compiler-input
77 passed; 0 failed

cargo test --offline -p skiff-compiler-source \
  dependency_analysis::tests::package_and_service_aliases_select_the_same_package_owned_type
1 passed; 0 failed

cargo test --offline -p skiff-compiler-source \
  expression_type_model::contract_call_typing::tests
3 passed; 0 failed

cargo check --offline -p skiff-compiler-input -p skiff-compiler-source
passed

git diff --check
passed
```

完整`skiff-compiler-source`为`215 passed; 6 failed`。6个失败均是本分支进入F164前已存在的普通Package
interface fixture断面，统一报`pkg.Reader`或`agent.llm.Client is not an interface`；F164涉及的service
schema consumer、contract call typing、callable effects与Package/service alias identity测试均通过。

完整`skiff-compiler`仍按任务链预期停在尚未迁移的lowering：
`compiler/lowering/src/executable_type_projection.rs`仍匹配已删除的`PackageTypeRef::Contract`。该断面不属于
F164的input/source范围。
