# P5-F445H I7 P8 D3 Stream argument and response sink refinement

状态：

```text
DOCS_ONLY
PRODUCTION_WRITE = NO
```

## 1. Parent and baseline

- 直接父节点：
  `P5-F445H-I7-P8-S1-package-direct-http-stream-registry-closure-result.md`
- Skiff baseline：
  `44e83695d5d9e6559b3ac5f482b9faffd1f96cb3`
  （tree `6cc2284797d52a6d3549afb255eeaae6247a6915`）。
- AIHub可恢复checkpoint：
  `bdf7bd4adc59cd32d615e4f5498d3e764df4384e`
  （tree `324fe69f3ea58786b297e412da436c03b05d9656`）。

## 2. Scope

本节点只把S1的强制停止与后续只读差分固化为两个顺序执行的精确任务：

1. S2验证一个overlay-local stream producer的返回值作为dependency PackageDirect stream producer
   参数时的transport/association；
2. S3在S2最终GREEN后独立验证deferred dependency producer是否保留现有raw HTTP response sink。

不定位或修复production根因，不运行build/test/live/network/stable instance/Mongo/OAuth/browser，不修改
S1状态，也不把`unknown Stream value`预写为registry、heap、argument transport、overlay或response sink
根因。

## 3. Completion

- S1继续为`TASK_NOT_EXECUTABLE / S1_COMPLETE=NO`；
- S2/S3各自拥有真实Router + `kind:test` fixture、精确trace字段、串行对照、最小候选owner与停止条件；
- DAG更新为`T -> S1 diagnostic -> S2 -> S3 -> I resume -> X -> J`；
- I只在S3明确给出`I_RESUME_UNBLOCKED=YES`后恢复；
- X/J覆盖新的argument transport和response sink证据；
- 禁止新增registry、协议、schema、compiler、Router、test-runner、std或Internals production机制。

执行结果：
`P5-F445H-I7-P8-D3-stream-argument-response-sink-refinement-result.md`。
