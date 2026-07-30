# P5-F325 Eval request trace fixture closure

状态：Completed。结果见
`P5-F325-eval-trace-fixture-closure-result.md`。

## 直接父节点

- schema/heap fixture result：
  `P5-F323-eval-fixture-schema-and-heap-closure-result.md`

父节点已经关闭19个schema和1个heap blocker。本任务只关闭随后暴露的一个typed-throw fixture trace缺口；
两个generic WebSocket blocker不属于本任务。

## DAG位置与写入范围

- 节点：F320 finding wave低风险机械fixture。
- 只允许：
  - `runtime/eval/src/assembly_execution/ordinary/tests.rs`
  - `runtime/eval/src/assembly_execution/ordinary/test_runtime.rs`
- 禁止修改`ProgramExecutionContext::next_exception_correlation`或任何production validation；
  禁止修改WebSocket、representation、service error core及权威设计。

## 完成标准

- `inline_effect_typed_throw_is_caught_by_exact_linked_nominal_type`使用显式非空、稳定的test request trace；
- trace必须从真实`ActorCapabilityApi::trace_id()`输入进入`ProgramExecutionContext`，不能在断言前后手工篡改
  exception或放宽empty-trace拒绝；
- fixture断言精确`traceId`以及由它派生的`errorId`前缀；
- 不需要throw的现有fixture可以继续使用无trace runtime；若新增with-trace test helper，不能全局改变缺trace负例；
- targeted test PASS，完整eval只应剩两个既知generic WebSocket失败。

```bash
cargo test -p skiff-runtime-eval --lib inline_effect_typed_throw_is_caught_by_exact_linked_nominal_type -- --exact
cargo test -p skiff-runtime-eval --lib -- --list
cargo test -p skiff-runtime-eval --lib --no-fail-fast
git diff --check
```

完整eval发现新独立失败时记录并停止，不修改授权外文件。不运行workspace/root/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f325-eval-trace-fixture`
- branch：`codex/p5-f325-eval-trace-fixture`
- 风险：低；新的一次性Agent，5分钟内修改；
- 提交并返回trace输入/断言、targeted与完整eval剩余失败；
- 不push、不承接WebSocket或acceptance。
