# P5-R29：F23D Real Smoke Third Reacceptance

权威设计为
`doc/architecture/package-service-contract-deployment.md`：

- §2“不变量”第4、5、8、9、10条；
- §5“ServiceDeployment”中Ingress只绑定ContractOperationId、显式operation mapping与完整dependency binding条款；
- §6.2“Service boundary call”中调用必须经过service dispatcher并切换activation owner的条款；
- §7“Linkable、Recoverable 与 Callback Capability”中callback capability owner/lifetime及runtime capability table条款；
- §12“RuntimeAssembly 与扩容”中每个environment单一active assembly、replica加载完整同一assembly以及health/atomic reload
  可观测条款；
- §14“Fail-closed 条件”中dependency、identity、callback/native adapter与ActivationContext错误不得退化或猜测的条款。

DAG节点为R29，依赖同一exact candidate上的I27 PASS；PASS完成F23D并解除R24。风险等级高，验收分组为F23D唯一真实
WebSocket生产路径。它是同一完整探针的第三次运行，仅因R28后已执行D38跨层剩余范围审计、F27A/B/C批量修复并由I27
combined PASS才获准执行。
冻结的production candidate为commit `3987923cb9abc5c852a4d8d9d16d347c5873138f`、tree
`f7457b1d11a43406763184e8ff220277d6ac6049`、Cargo.lock blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`；随后只允许I27/R29合同文档提交。R29必须复用I27回报的exact
HEAD/tree/Cargo.lock，不得自行换候选。

必须使用未参与F23/F24/F25/F27及R26/R28/I27的全新只读Agent。在I27验证的同一exact clean candidate仓库根目录只执行
下面一条命令一次：

```bash
node scripts/run-package-service-ecosystem-smoke.mjs --probe skiff-cutover --replicas 1 --checkout "$PWD"
```

不得编辑、提交、修复或重跑combined/full/I16/Host/stable。必须观察normal source→canonical std store closure→compiler/
deployment/assembly→strict receipt→activation generation1→exact readiness→single WS connect/receive→Event/Result
materialization→native direct-send marker，且cleanup完整；禁止fake/protocol peer/业务retry。

第一行只给`R29 PASS`或`R29 FAIL`。PASS须给出exact commit/tree/Cargo.lock、receipt/activation/readiness/WS/native marker及cleanup
的有界证据，完成F23D并解锁R24；FAIL须给第一错误和F26A bounded diagnostic，禁止重试。证据仅对I27相同exact
commit/tree/Cargo.lock和本次隔离环境有效；相关代码、fixture、依赖、generated artifact或运行环境状态变化都会使其失效。
