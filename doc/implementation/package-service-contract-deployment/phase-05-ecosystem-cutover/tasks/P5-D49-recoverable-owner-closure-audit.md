# P5-D49：Recoverable Owner Closure Audit

DAG节点D49，依赖I02C在冻结production commit
`ad847f7254521d1dd4679a4f8af72b2c88753310`、tree
`f0a33cc750025916df7b303e2f07b9db3f2e9c6d`的FAIL。权威设计为
`doc/architecture/package-service-contract-deployment.md` §1–§15。

三个全新只读分片可并行：

- D49A：审计`runtime/eval` canonical assembly recoverable concrete-owner lookup、identity与去重规则。
- D49B：追踪I02 fixture从package/deployment/assembly到execution image的package id来源，确定重复是合法多对象表示、
  producer重复还是consumer错误折叠。
- D49C：枚举现有单元/集成覆盖与最小非E2E探针，要求能复现duplicate并验证修复后exact owner及关键负例。

分片只返回代码事实、设计追溯、唯一owner候选、最小修复边界与测试建议，不编辑、不提交、不运行完整I02/R05/
instance/stable/full gate。D49汇总必须解释三个分片是否一致，批量更新后续修复DAG；不得直接作I02/R02 verdict。
