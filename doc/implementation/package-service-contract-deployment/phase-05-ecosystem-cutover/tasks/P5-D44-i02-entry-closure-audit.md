# P5-D44：I02 Entry Closure Audit

权威设计为
`doc/architecture/package-service-contract-deployment.md` §1–§15；执行完成态来自`P5-I02-skiff-combined-probe.md`。

DAG节点D44，依赖I02 exact FAIL。当前旧ecosystem smoke只覆盖single-generation activation+WS marker；D44只读闭合
I02剩余证据入口或最小implementation DAG，不作I02/R02 verdict。

全新只读Agent在production candidate
`c59b4baf9752147cc49c141d89642d8b7f5aa507`建立矩阵：

- authoring receipt、activation prepare/admit/commit与result中activationId/generation/assembly/replica的现有owner；
- tampered candidate如何经真实production prepare/reject/abort，证明committed tuple、旧Host结果不变及pending/staged归零；
- request path artifact I/O=0的现有health/diagnostic/test owner；
- capabilities、binary assembly frames及actor/spawn typed response是否已有真实isolated入口，可否组合而不复制协议peer；
- 哪些I02条款已由R05B精确覆盖且不应重复，哪些仍被single-generation runner遮挡；
- 固定one-replica命令、cleanup/deadline/temporary Cargo target、输出ledger字段；
- 若缺入口，冻结最小scripts/test-infrastructure节点、必要production diagnostic节点、互斥写入owner、direct tests、cheap
  combined及证据失效面。

只允许`rg`、`git log/show/diff`及源码/fixture/既有测试静态读取；禁止编辑、提交、构建、测试、启动
Router/runtime/instance、运行smoke或操作stable。不得用fake registry/protocol peer/manual emitter替代production入口。
若需要改变activation/abort、四对象、control wire或公共diagnostic语义，标记设计决策；否则归implementation owner。
