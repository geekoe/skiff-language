# P5-F159：Package Schema Artifact Model

状态：Ready

## 直接父任务

- `P5-H32-package-owned-type-cutover.md`

## 范围

只修改Skiff公共artifact model、identity canonicalization及其聚焦测试。不得修改compiler projection、
dependency importer、deployment、runtime或consumer service。

## 必须实现

1. 用`PackageSchemaTypeId`替代service-owned `ContractTypeId`的canonical boundary nominal identity。
2. 定义严格序列化的：
   - `PackageSchemaTypeRecord { packageId, stableSchemaKey, packageSchemaTypeId, canonicalDescriptor }`
   - `PackageSchemaIndex { packageId, packageSchemaIndexIdentity, types }`
   - `PackageTypeRequirement { packageId, requiredTypeIds }`
3. `PackageArtifact`引用自己的PackageSchemaIndex及所需逐类型记录；字段形状必须让artifact store能够按
   `PackageSchemaTypeId`独立寻址和去重。
4. `ServiceContract`删除内嵌`boundary_schema`，改为按Package分组的精确`PackageTypeRequirement`。
5. operation/type descriptor中的名义引用直接携带owner package id与`PackageSchemaTypeId`；不得存在
   service-owned nominal variant或`PackagePublic`临时variant。
6. identity preimage遵守权威设计：
   - TypeId包含packageId、stableSchemaKey、canonicalDescriptor；
   - version/build/service/deployment/nameability/publicPath不进入TypeId；
   - index按stableSchemaKey排序；
   - ServiceProtocolIdentity只包含operation实际可达的type ids，不包含index identity。
7. 第一版递归schema必须fail closed；公共API不能提供看似支持循环哈希的构造路径。

## 验证

- artifact-model聚焦测试与identity聚焦测试；
- strict serde缺字段/多字段负例；
- 同type跨version/build身份相同；
- 无关index entry变化不改变只引用既有type的protocol identity；
- descriptor、owner package或stable key变化改变type id；
- `git diff --check`。

## 交付

- 独立提交；
- result文档记录修改、命令、通过/失败和后续迁移所需的精确编译错误面；
- 不以大规模fixture机械修复掩盖尚未迁移的下游owner。

