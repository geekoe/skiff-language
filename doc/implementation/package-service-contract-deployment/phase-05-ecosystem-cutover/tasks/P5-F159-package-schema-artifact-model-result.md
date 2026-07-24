# P5-F159：Package Schema Artifact Model Result

状态：Completed

## 直接父任务

- `P5-F159-package-schema-artifact-model.md`

## 交付

- 公共名义边界类型身份从service-owned `ContractTypeId`硬切为Package-owned
  `PackageSchemaTypeId`；`ContractTypeRef`和`PackageTypeRef`只保留携带
  `packageId + stableSchemaKey + packageSchemaTypeId`的名义引用。
- 新增严格wire模型：
  - `PackageSchemaTypeRecord`
  - `PackageSchemaIndex`
  - `PackageTypeRequirement`
  - 独立寻址所需的index/type record refs
- `PackageArtifact`只引用content-addressed schema index和逐类型record，不内嵌descriptor payload。
- `ServiceContract`删除`boundarySchema`，只保留按Package分组且排序去重的
  `packageTypeRequirements`；wire schema升为v3。
- `PackageSchemaTypeId`的canonical preimage只包含package id、stable schema key和canonical
  descriptor；version、build、service、deployment、nameability和public path均不参与。
- `PackageSchemaIndexIdentity`按`BTreeMap`的stable schema key顺序计算；index identity不进入
  `ServiceProtocolIdentity`。
- Package schema record validator先拒绝self/SCC，再校验逐类型identity、owner/key和传递闭包。
- WebSocket ingress schema验证不再读取ServiceContract内嵌schema，必须显式接收resolved
  `PackageSchemaTypeRecord`集合；缺record、非requirement引用、owner/key/id不匹配或cycle都fail closed。
- 删除旧service-owned schema canonicalization、graph validator及其fixture；未保留旧wire兼容层。

## 验证

通过：

```text
cargo test -p skiff-artifact-model --lib
109 passed; 0 failed

cargo test -p skiff-artifact-identity --lib
71 passed; 0 failed

git diff --check
passed
```

聚焦测试覆盖：

- record/index/requirement缺字段与多字段strict serde；
- 同一descriptor跨version/build保持相同type identity；
- descriptor、owner package、stable key变化改变type id；
- 无关index entry改变index identity，但不改变只引用既有type的protocol identity；
- requirement必须按package及type id排序去重；
- recursive package schema fail closed。

## 预期下游编译断面

`cargo check --workspace`按预期失败；本节点没有越界修复下游owner。首批精确断面为：

1. `runtime/model/src/callback_projection.rs`
   - 仍导入`ContractTypeId`；
   - 仍匹配`ContractTypeRef::Contract`。
2. `compiler/contract/src/compile.rs`
   - 仍调用已删除的`contract_type_id`和`normalize_contract_definition_surface`；
   - 仍构造`ContractSchemaType`、`ContractTypeId`与`ServiceContract.boundary_schema`。
3. `compiler/contract/src/projection.rs`
   - 仍生成service-owned type ids；
   - 仍匹配`PackagePublic`/`Contract`；
   - callback仍读取`interface_type_ids`。
4. `compiler/projection/src/package_artifact/boundary/types.rs`
   - 仍把`PackageTypeRef::Contract`投影为`ContractTypeRef::contract`；
   - 仍生成`PackagePublic`；
   - `PackageArtifact`构造尚未提供schema index/type record refs。
5. `compiler/input/src/contract_dependencies/{error,index}.rs`
   - importer仍以`ContractTypeId`索引`ServiceContract.boundary_schema`。

这些错误分别对应父任务DAG中的compiler projection、dependency import和runtime迁移节点，不能通过
恢复旧variant、空schema或兼容字段绕过。
