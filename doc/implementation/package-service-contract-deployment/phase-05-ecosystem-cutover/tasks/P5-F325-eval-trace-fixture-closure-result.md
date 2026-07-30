# P5-F325 Eval request trace fixture closure result

状态：PASS。

实现提交：`d3a7bf8f`。

## 结果

- 新增按需使用的`actor_context_with_trace`；既有无trace helper保持，缺trace负例没有被全局掩盖。
- typed-throw fixture使用`test-trace:inline-effect-typed-throw`，经真实
  `TestActor::trace_id()`进入`ProgramExecutionContext::new`。
- fixture精确断言该`traceId`以及
  `test-trace:inline-effect-typed-throw:local-error:`派生前缀。
- 没有修改production trace校验、exception或WebSocket。

## 验证

- targeted exact test：1/1 PASS。
- eval list：154，非零。
- 完整eval：152/154 PASS；仅剩两个既知generic WebSocket source-inline blocker。
- crate `rustfmt --check`与`git diff --check`：PASS。

