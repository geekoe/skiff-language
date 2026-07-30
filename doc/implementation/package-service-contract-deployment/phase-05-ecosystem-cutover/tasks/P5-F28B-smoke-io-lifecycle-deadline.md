# P5-F28B：Smoke I/O Lifecycle Deadline

权威设计为
`doc/architecture/package-service-contract-deployment.md` §12“RuntimeAssembly 与扩容”中health/atomic reload可观测条款，
以及§14“Fail-closed 条件”。本任务只约束验收harness资源生命周期，不改变activation/health/WS production语义，不增加业务
retry。

DAG节点F28B，依赖F28A以避免共享Node test冲突；完成后解除I28 lifecycle分片。风险高，验收分组为R29 isolated cleanup。
进入状态必须是F28A已合流的exact integration HEAD。

写入边界仅：

- `scripts/lib/package-service-ecosystem-smoke-real.mjs`；
- 必要时`scripts/lib/package-service-authoring.mjs`的可选signal/deadline transport；
- 两者专用直接Node tests，含`package-service-ecosystem-smoke-real.test.mjs`。

要求：

- activation fetch、WebSocket opened与close共享本次isolated run的有界deadline/AbortSignal；
- timeout/abort保留最先primary error并保证`runTest`返回，使outer stop/down/port/lease/workspace cleanup可开始；
- close超时必须有owner明确的`terminate()` fallback，不能无限等待close事件；
- 正常路径仍只创建一次业务WS，不重试activation或业务请求，不吞掉F26A diagnostic；
- 直接负探针覆盖永不返回activation、永不open、永不close，并断言abort/terminate及outer cleanup都发生。

只运行专用Node tests/syntax/diff-check；禁止Cargo、真实smoke、combined/full/I16/Host/stable。一个clean commit。smoke
lifecycle/isolated owner接口变化使证据失效。
