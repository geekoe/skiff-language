# P5-F445H I7 P8 D0 HTTP entry 测试权威收敛

状态：

```text
AUTHORITY_READY
PRODUCTION_IMPLEMENTATION_STARTED = NO
```

## 1. Parent and baseline

- 直接事实父节点：
  `P5-F445H-I7-M6-aihub-post-g2-diagnostic-result.md`
- 唯一顶层架构事实源：
  `../../../../architecture/package-service-contract-deployment.md`
- 测试语义owner：
  `../../../../reference/testing.md`
- runner隔离owner：
  `../../../../architecture/test-runner-runtime-isolation.md`
- 标准库边界：
  `../../../../reference/std-surface.md`
- Skiff baseline：
  `3a87d37f81a04c249f308b311bd91dcfdf3a8aa3`
  （tree `eafc29e952f6b5170e4f5faca4e5d181b3ace9f6`）
- Internals baseline：
  `9c3bdc82c4a43e575ea627357c05f54dbc0400a8`
  （tree `c3f159a397cd3c2b316a502ce945d8a935a9c2c3`）

## 2. Design scope

本节点只把M6剩余四条AIHub失败收敛为first-class HTTP entry test能力，并更新权威文档。完成语义是：

1. test service显式`http.yml`和`*.test.skiff` wrapper；
2. 测试源码复用`std.http.request`/`std.http.stream`与普通绝对URL；
3. runner提供动态business ingress URL和当前case唯一service/version；
4. Router走普通business HTTP路由；
5. self-ingress子请求与父case共享inline-effect registry，父case唯一finalize；
6. 第一版同case不并发self-ingress；
7. stream复用现有disconnect、cancel与backpressure链；
8. AIHub按完整body/SSE event断言，不按网络chunk断言。

明确不新增标准库surface、语言关键字、特殊URL、test session header/token、Router旁路、
runtime wire/schema或artifact格式。compiler、std和File IR默认零改动。

## 3. Execution boundary

本任务只改权威文档、decision ledger和P8任务合同；不改production/test code，不运行build、test、
live、network、stable instance、Mongo、OAuth或browser。

后续owner必须先在精确baseline做零worktree只读预检。任何production改动都要由一条当前RED路径证明；
若现有能力已能闭环，对应节点返回`NO_PRODUCTION_CHANGE`，不得为满足任务名制造改动。
