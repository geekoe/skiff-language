# P5-F161：Package Schema Compiler Projection

状态：Ready

## 直接父任务

- `P5-F159-package-schema-artifact-model-result.md`

## 范围

修改compiler PackageArtifact boundary projection与compiler/contract。不得修改store、compiler dependency
import、deployment或runtime。

## 必须实现

- 从typed Package public API graph及schema closure生成`PackageSchemaTypeRecord`和`PackageSchemaIndex`，
  PackageArtifact写入精确index/type record refs。
- operation中的命名类型直接使用
  `packageId + stableSchemaKey + PackageSchemaTypeId`；删除`PackagePublic`、`Contract`和
  `canonicalize_service_owned_*`生产路径。
- 同时覆盖本package、普通package dependency和official `skiff.run/std`类型；std HTTP类型必须引用
  std Package类型，不能再结构展开。
- 当前frontend没有稳定closure-only声明键时必须明确fail closed，不得用文件路径、遍历序号或display
  string造键。
- ServiceContract只选择Available operations并计算其传递可达Package类型闭包，生成精确
  `PackageTypeRequirement`；不复制descriptor、不携带index identity。
- compiler/contract删除旧definition-owned boundary schema编译路径；若authoring DTO已无产品用途则删除，
  不保留双轨。
- Package schema图在identity分配前拒绝self/SCC。

## 必须测试

- 真实源码PackageArtifact→ServiceContract，不能只手造DTO。
- 两个service引用同一个Package类型得到同一PackageSchemaTypeId。
- package version、service id、implementation build变化不改变type id。
- 未被operation引用的Package类型变化只改变index/build，不改变protocol。
- 被引用descriptor/child/owner/key变化改变type id与protocol。
- std HTTP request/response/stream event保留`skiff.run/std` owner；非官方同名fail closed。
- package dependency named type能够进入operation；缺schema source/closure-only稳定键/SCC fail closed。
- compiler projection与contract聚焦测试、`git diff --check`；独立提交并写result。

