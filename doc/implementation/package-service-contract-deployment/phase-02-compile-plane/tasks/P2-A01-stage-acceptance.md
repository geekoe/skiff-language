# P2-A01：Independent Stage Acceptance

## 角色

未参与Phase02开发的独立只读验收Agent。不得修改文件、创建commit或替开发Agent解释实现。

## 输入

- 原始用户目标。
- 唯一设计：`doc/architecture/package-service-contract-deployment.md`。
- 总纲、Phase02 plan及P2-T01–T07全部任务文件。
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
7. package-test/test-runner复用production PackageArtifact；legacy adapter只转换shape且有删除owner/gate。
8. T07证据对应最终commit；高风险schema/identity/effect/lowering有独立代码和负例证据。

## 输出

第一行`PASS`或`FAIL`。FAIL列blocking issue、设计/任务证据、production代码证据、影响和建议owner；另列
non-blocking follow-up、已运行聚焦命令、未覆盖动态风险。已有昂贵gate仍有效时不机械重跑。
