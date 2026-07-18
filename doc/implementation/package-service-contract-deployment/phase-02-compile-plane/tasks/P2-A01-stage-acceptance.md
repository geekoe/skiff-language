# P2-A01：Independent Stage Acceptance

## 角色

未参与Phase02开发的独立只读验收Agent。不得修改文件、创建commit或替开发Agent解释实现。

## 输入

- 原始用户目标。
- 唯一设计：`doc/architecture/package-service-contract-deployment.md`。
- 总纲、Phase02 plan及P2-T01–T07、P2-R02–R13全部任务文件。
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

## 输出

第一行`PASS`或`FAIL`。FAIL列blocking issue、设计/任务证据、production代码证据、影响和建议owner；另列
non-blocking follow-up、已运行聚焦命令、未覆盖动态风险。已有昂贵gate仍有效时不机械重跑。
