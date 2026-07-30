# P5-F164：Package Schema Consumer Import

状态：Ready

## 直接父任务

- `P5-F163-package-schema-projection-recovery-result.md`

## 范围

修改compiler/input与compiler/source中service dependency类型导入、解析和materialization。不得修改
deployment或runtime。

## 必须实现

- `ResolvedContractDependency`不再从`ServiceContract.boundary_schema`读取类型；它必须接收并验证contract
  `PackageTypeRequirement`对应的精确`PackageSchemaTypeRecord`闭包。
- service alias只选择operation/API module；名义类型身份保持owner Package的
  `packageId + stableSchemaKey + PackageSchemaTypeId`。
- 删除`ContractTypeId`索引与`TypeRefIr::ServiceSymbol`作为service-owned identity的路径。若语法仍允许
  `serviceAlias.Type`，解析结果必须与直接package import的同一owner类型精确相同。
- descriptor materialization复用普通Package nominal类型机制；assignability、interface substitution、
  stream item与callback不得按display string或结构相同放宽。
- 缺record、额外record、owner/key/hash/closure不匹配、contract未require的type、未公开type均fail closed。

## 验证

- compiler input/source恢复编译及聚焦测试。
- 同一Package类型经package alias与service alias导入后身份和赋值完全一致。
- service alias重命名、provider service id/version/build变化不改变Package type identity。
- operation调用、server stream item、typed error和callback保留Package schema ref。
- 真实store-backed ServiceContract + schema records导入；不能只手造DTO。
- `git diff --check`；独立提交并写result。

