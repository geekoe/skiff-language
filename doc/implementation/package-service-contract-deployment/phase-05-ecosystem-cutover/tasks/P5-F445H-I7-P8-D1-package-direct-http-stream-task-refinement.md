# P5-F445H I7 P8 D1 PackageDirect HTTP stream task refinement

状态：

```text
DOCS_ONLY
PRODUCTION_WRITE = NO
```

## 1. Parent and baseline

- 直接父节点：
  `P5-F445H-I7-P8-T-http-entry-combined-probe-result.md`
- 唯一顶层架构事实源：
  `../../../../architecture/package-service-contract-deployment.md`
- Skiff baseline：
  `ff6418f5a43ee503608cf8f54512bd9f53a47a74`
  （tree `a6ea21c20231e40db69960f70cc6850a7723f871`）
- I可恢复Internals checkpoint：
  `bdf7bd4adc59cd32d615e4f5498d3e764df4384e`
  （tree `324fe69f3ea58786b297e412da436c03b05d9656`）

## 2. Scope

本节点把P8-S只读探查结论固化为可执行S1任务，并更新P8 DAG、I blocker、X/J输入与result ledger。
只允许澄清既有package direct、service boundary、HTTP parent/child request语义，不改变公共API、ABI、
wire、schema或测试协议。

本节点不定位或修复production根因，不运行build、test、live、network、stable instance、Mongo、
OAuth或browser。I checkpoint中的`unknown Stream value`只能记录为观察结果，不能写成已知registry根因。

## 3. Completion

- S1拥有一个有界、可执行、先RED后修复的Runtime/Host合同；
- DAG为`T -> S1 -> I resume -> X`；
- I checkpoint保持可恢复且在S1前不继续吞并Skiff修复；
- J覆盖新增的Codex Relay全部default isolated tests与official packages全部default offline tests，
  并保持Account既有receipt/assembly checks；
- 文档反向检查不含新registry、协议、header、schema、compiler、Router或test-runner方案。

执行结果：
`P5-F445H-I7-P8-D1-package-direct-http-stream-task-refinement-result.md`。
