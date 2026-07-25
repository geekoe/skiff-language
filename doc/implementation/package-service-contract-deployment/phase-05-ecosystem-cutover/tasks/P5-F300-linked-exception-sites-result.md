# P5-F300 Linked exception instruction facts结果

状态：Implemented checkpoint。

任务提交：`b10fd6ccba84be31e2be3d38a9503bfe61b5c875`。

集成提交：`91e0e48f3d6458b9e2967f12d9bf82a83f01a81b`。

## 直接任务与权威链

- `P5-F300-linked-exception-sites.md`
- 任务继续引用F299第一次阻塞结果、F298、F286与F297父链。

## 结果

- `LinkedStmtIr::Throw`、`LinkedExprIr::Throw`和linked `CallIr`持有required
  `InstructionSourceSite`；
- `LinkedExprIr::Catch.catch_type`是required `LinkedTypeRef`，删除Option、serde default与
  optional linking；
- file conversion逐值复制artifact site与required catch type；
- assembly code linker无条件链接catch type；
- linked JSON缺少throw/call site或catch type时严格拒绝；
- 没有compatibility default、synthetic fallback、site重写、display/shape推断或F297
  AppliedNominal退化。

## 验证

- linked-program list/full：PASS，31/31；
- linker list/full：PASS，40/40；
- `git diff --check`：PASS；
- optional linked catch与conversion catch `.as_ref().map`降级反搜：production零命中。

下游runtime eval/host/driver仍需给runtime-generated calls显式Synthetic site，并让eval消费required
throw site/catch type；这些由F299及后续fixture owner负责。本检查点解除重新派发F299。

