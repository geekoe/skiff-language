# P5-H32：Package-owned Boundary Type Cutover

状态：Implementation Checkpoint

## 直接父任务

- `P5-H31-r05-batch-handoff.md`

## 决策增量

用户已纠正旧实现模型：Service首先是Package，boundary命名类型始终由声明它的Package拥有。
ServiceContract只选择operations并引用这些Package类型，不能复制descriptor、重写成service-owned
`ContractTypeId`或在HTTP位置展开匿名结构。

后续用户决策：第一版所有boundary可达named types必须在owner Package的`api.yml`显式公开；不支持
closure-only内部命名类型进入boundary。compiler必须拒绝它，不能从模块/文件路径或遍历顺序造稳定键。

完整规范见：

- `../../../../../architecture/package-service-contract-deployment.md`
- `../../../../../reference/publication.md`
- `../../../../../reference/static-semantics.md`
- `../../../../../reference/std-surface.md`

设计评审结果见同目录`P5-D92-package-owned-type-design-review.md`；三个原阻塞项均已闭合。

## 当前错误模型

当前代码仍把以下结构作为canonical：

- `ContractTypeId`与`ContractTypeRef::Contract`；
- `ServiceContract.boundary_schema: ContractTypeId -> ContractSchemaType`；
- `ContractTypeRef::PackagePublic`只允许在Package projection暂存，随后被重写为service-owned类型；
- dependency importer从ServiceContract内嵌schema重建类型；
- runtime boundary、callback和ingress校验直接读取contract-owned schema map。

这不是HTTP service contract的局部错误，而是Package、Service、dependency import和runtime共享的owner错误。
Skiff尚未发布，不保留旧artifact wire兼容层。

## 实现DAG

1. `P5-F159-package-schema-artifact-model.md`
   - 建立PackageSchemaTypeId、PackageSchemaTypeRecord、PackageSchemaIndex与PackageTypeRequirement；
   - ServiceContract移除内嵌boundary schema；
   - 所有公共identity canonicalization和严格wire测试闭合。
2. compiler projection
   - 从typed Package API生成逐类型record/index；
   - operation保留PackageSchemaTypeId；
   - ServiceContract只收集实际可达type ids，且无关Package类型变化不改变protocol identity；
   - 真实PackageArtifact→ServiceContract测试必须覆盖std HTTP类型。
3. dependency import与artifact resolution
   - importer按content-addressed PackageSchemaTypeRecord闭合类型；
   - 不读取provider源码、active deployment或同version猜测；
   - 缺record、owner/key/hash或closure不匹配时fail closed。
4. deployment/runtime/ingress
   - admission、value plan、callback和typed ingress改读resolved Package schema closure；
   - 删除service-owned schema假设与兼容分支。
5. Registry、Codex Relay及其余真实service重验
   - 只有共享链路闭合后恢复；不得在consumer中手写HTTP结构或wrapper规避。

## Gate

- 公共模型、compiler、importer和runtime各有聚焦测试；
- 至少一条真实源码链证明Package类型经PackageArtifact进入ServiceContract引用，再由consumer导入；
- 新增未被service operation引用的Package类型不改变ServiceProtocolIdentity；
- 被引用类型descriptor变化必须改变PackageSchemaTypeId和ServiceProtocolIdentity；
- std HTTP类型保持`skiff.run/std` owner，不内联、不复制；
- 全仓搜索不得残留“service projection must replace Package type with service-owned ContractTypeId”的生产规则。
