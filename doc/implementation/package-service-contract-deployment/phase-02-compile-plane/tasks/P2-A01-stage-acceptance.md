# P2-A01：Independent Stage Acceptance

## 角色

未参与Phase02开发的独立只读验收Agent。不得修改文件、创建commit或替开发Agent解释实现。

## 输入

- 原始用户目标。
- 唯一设计：`doc/architecture/package-service-contract-deployment.md`。
- 总纲、Phase02 plan及本阶段全部任务文件，包括T03A–J、T04A–D、T05C1–13与R10A–I。
- integration branch最终commit与T07证据表。

## 必验条款

1. 当前生产compiler确实只有package code pipeline，旧Publication compiler owner不是改名或隐藏adapter。
2. ServiceContract可在无provider时产生；provider和consumer只凭contract独立compile。
3. PackageArtifact与ServiceContract没有共同aggregate、PublicationAbiUnit/ServiceUnit或provider/deployment
   泄漏。
4. effect/provenance是sound fixed point；Unknown/复杂行为fail closed，简单safe callable可Available。
5. direct package call的alias/mutation不被service boundary规则禁止。
6. 实际service call生成ServiceRequirement/ServiceCallRef；未调用声明不生成runtime edge，consumer artifact
   无provider target。
7. compiler 不产出 PublicationAbiUnit/PackageUnit/ServiceUnit/serviceAssembly，不存在 legacy/
   compatibility adapter、空 runtime holder、dual-write、fallback 或 checker allowlist。
8. T07证据对应最终commit；高风险schema/identity/effect/lowering有独立代码和负例证据。
9. compiler integration fixtures 没有通过空/fake contract、provider inference 或新聚合 builder 恢复
   旧 service=code+deployment 模型，原 test targets 的覆盖不是无证据删除。
10. canonical contract 的 discriminator/branch tag、map key identity、builtin grammar、nullable normalization
    与当前 recursion policy 全部进入 typed validation/identity，不依赖未来 JSON renderer 补语义。
11. 当前 service CLI/watch/runtime 不可用被明确记录为阶段断链，没有为让它们继续运行
    而引入 provider inference 或兼容代码。
12. `alias.Type`解析为validated ServiceContract的public-nameable ContractTypeId，`alias/operation(...)`按同一
    descriptor在source阶段完成参数/返回检查；package/contract alias冲突在trust boundary失败。
13. exact contract-aware callable signature沿唯一compiled/projection-input路径进入PackageArtifact；没有
    File IR/display string反推或blanket Local producer，lowering也没有第二份contract operation index。
14. 全部source executable从唯一exact facts投影File IR execution representation；contract leaf只成为opaque
    unknown且不携带identity。不存在dot dependency-call兼容、旧remote-only AST owner或ServiceSymbol fallback。
15. interface operation拥有source exact facts，impl conformance比较ContractTypeId而非alias-shaped
    ServiceSymbol；interface File IR同样只含opaque execution representation。PackageArtifact public-instance
    projection只发现execution target，compiled/projection-input不能重算conformance，projection不能从File IR
    生成/比较OperationAbiRef或semantic signature。
16. expression exact projection直接消费resolved IR和完整PackageTypeRef sidecar；LocalType debug/display不回流
    source parser，Map keys、单/双binding for-in、local generic与nullable narrowing不丢Local/Contract/container/
    nullable事实；缺失contract sidecar与unsupported inline shape继续fail closed。
17. terminal boundary checker精确冻结fallible projection handoff、callable/public-instance DTO、canonical public-path
    helper与ProjectionInput字段；test-only排除来自`#[cfg(test)]`模块可达性，production同类import仍被拒绝，
    不存在通配allow-list、路径特例或known-violation ledger。

## 输出

第一行`PASS`或`FAIL`。FAIL列blocking issue、设计/任务证据、production代码证据、影响和建议owner；另列
non-blocking follow-up、已运行聚焦命令、未覆盖动态风险。已有昂贵gate仍有效时不机械重跑。
