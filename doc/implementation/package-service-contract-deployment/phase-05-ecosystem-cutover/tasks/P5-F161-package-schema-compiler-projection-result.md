# P5-F161：Package Schema Compiler Projection Result

状态：Blocked

## 直接父任务

- `P5-F161-package-schema-compiler-projection.md`

## 结论

本节点不能在既定范围内形成任务要求的真实源码链。没有恢复
`PackagePublic`、`Contract`、`boundarySchema`或service-owned类型模型，也没有提交以路径、遍历序号、
display string或HTTP名字特判生成身份的替代实现。

## 精确上游事实

### 1. compiler projection拿不到依赖Package schema实体

当前`PackageArtifactProjectionInput`只携带：

- `Vec<PackageRequirement>`；
- `Vec<ContractRequirement>`；
- `Vec<ServiceRequirement>`；
- service call refs。

`PackageRequirement`只有依赖坐标、版本和ABI期望。它不携带已验证的
`PackageSchemaIndex`/`PackageSchemaTypeRecord`，`ProjectionView`也只有当前Package的typed File IR。

因此，`TypeRefIr::PackageSymbol { package: Dependency { ... } }`无法被投影为任务要求的
`packageId + stableSchemaKey + PackageSchemaTypeId`。projection既不能验证依赖owner/key/id，也不能读取
descriptor闭包。仅凭symbol path计算id会遗漏canonical descriptor，属于伪身份。

F160定义的store解析入口可以提供已验证实体，但F161当前输入模型没有接收该解析结果的字段或resolver。
把依赖schema读取逻辑直接塞进projection会越过本任务明确排除的compiler dependency import边界。

### 2. closure-only声明没有稳定键事实

当前typed graph对本包非公开名义声明只提供`module_path + symbol`或`type_index`。没有frontend分配的、
与文件路径和遍历顺序无关的package-local declaration identity/stable schema key。

所以projection可以安全使用公开API path作为公开类型的`stableSchemaKey`，但一旦公开descriptor或operation
闭包到达非公开名义类型，只能按要求fail closed，无法完成任务同时要求的closure record生成。

### 3. std HTTP不能在compiler中继续特判成结构或伪Package记录

现有`canonical_http_boundary_type`返回compiler内置的结构描述；这不等于解析
`skiff.run/std` Package发布的schema record。用它在consumer package投影时计算一个“std type id”，会建立
第二个schema生产源，违反“std和普通Package走同一路径”及单一Package owner规则。

要满足本任务，std必须先由自身PackageArtifact产出并存储schema index/type records，再通过与普通依赖相同
的已验证bundle/resolver进入consumer projection。

### 4. ServiceContract精确闭包也需要resolved records

`PackageArtifact`只保存schema record refs，不内嵌descriptor。`project_service_api(service_id, package)`
当前只有PackageArtifact，无法沿operation根引用计算传递可达类型闭包。

若把当前PackageArtifact的全部record refs写入`packageTypeRequirements`，未被operation引用的类型变化就会
改变`ServiceProtocolIdentity`，直接违反F161验收条件。正确API必须额外接收已验证records或resolver。

## 需要补齐的前置契约

应先新增一个父级checkpoint，明确并交付：

1. compiler driver在依赖解析阶段通过F160入口取得已验证的Package schema bundle；
2. `PackageArtifactProjectionInput`接收按dependency绑定的只读schema bundle/resolver，std走同一路径；
3. service contract projection接收operation所需的resolved record resolver，而不是只接收refs；
4. frontend为closure-only名义声明提供package-local稳定声明键；在该事实落地前，语言规则可暂时明确限制
   boundary-reachable名义类型必须公开，并保持fail closed。

完成这些前置事实后，F161才能在不猜测、不复制descriptor、不恢复旧模型的前提下实现：

- Package schema DAG及逆拓扑identity分配；
- PackageArtifact精确index/type record refs；
- dependency/std named type投影；
- ServiceContract传递可达`PackageTypeRequirement`；
- 两个service共享同一Package type id的真实源码测试。

## 验证

基线确认：

```text
cargo check -p skiff-compiler-projection -p skiff-compiler-contract
failed as expected at the F159 hard-cut seam
```

首批错误与F159 result一致：旧compiler仍引用已删除的`ContractTypeId`、
`ContractTypeRef::{Contract,PackagePublic}`、`ServiceContract.boundary_schema`，且PackageArtifact构造尚未提供
schema refs。由于上述输入事实缺失，本节点没有用兼容字段或空schema掩盖这些断面。

```text
git diff --check
passed
```
