# P5-F308 Representation wrap shared model acceptance

状态：PASS。结果见
`P5-F308-representation-wrap-model-acceptance-result.md`。

## 验收输入

- 权威父链：`P5-F306-representation-constructor-carrier-audit-result.md`
- 实现任务：`P5-F307-representation-wrap-shared-model.md`
- 实现结果：`P5-F307-representation-wrap-shared-model-result.md`
- exact production candidate：`bd44c1b038ba48eb5a0f5288baa2234d174fec7a`
- integration merge：`3dbd2119f6899d781e8068d6a529f3a7d3c6a932`
- merge tree：`fd16e4a6b4fde777a584f14f85fe62977aebf98f`

## 角色与边界

独立只读高风险验收。不得修改/提交文件，不得接受开发总结作为结论，不运行workspace/stable/live。
给出唯一PASS/FAIL、blocking issues、non-blocking follow-up与证据。

## 必查

1. exact diff只触碰F307授权artifact-model/identity owner，无compiler/runtime/compat。
2. `RepresentationWrap`只有一个required wire shape；unknown/missing/null/legacy fields严格拒绝。
3. child ref scope与target contextual admission真实执行；仅plain/applied exact Representation，
   generic arity/nested args验证，所有非法kind/owner/PackageSchema fail closed。
4. visitor/identity preimage包含完整target与child；owner/argument/child tamper可检出。
5. generation精确为File IR v8/v6/v8，opcode v1与其它artifact/contract generation保持。
6. 没有record field模拟、display/static throw恢复、compat default/dual path或consumer实现。
7. 开发证据156/156、94/94对应exact candidate且selector非零。

可运行最小命名聚焦测试与只读反搜，不机械重跑两个完整suite。若现有测试无法证明关键负例，运行最窄
mutation/serde/admission探针并记录。

PASS解除representation compiler与linked consumer；FAIL列出精确blocker及失效证据面。
