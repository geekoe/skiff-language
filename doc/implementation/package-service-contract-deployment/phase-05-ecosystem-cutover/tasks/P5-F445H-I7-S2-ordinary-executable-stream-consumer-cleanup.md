# P5-F445H-I7-S2 普通可执行 stream consumer 监督与清理

## 1. 父任务与问题

直接输入：

- `P4-F14R-supervised-stream-consumption.md`：prepared producer 已有唯一 outer lease，
  native consumer 也会取得其 child；
- `P5-F445H-I7-S1-host-runtime-router-cross-layer-receipt-result.md`：真实 stream
  ingress/dispatch receipt 已闭合；
- `doc/architecture/package-service-contract-deployment.md` §6：stream producer/consumer
  必须携带显式 owner，不得依赖隐式全局状态。

当前缺口只在 Eval 内：当 prepared producer 的 `Stream` 参数传给普通 Skiff 可执行函数时，
consumer 内部的 `for in` 会创建独立 `StreamConsumerCleanup`。consumer 在 producer terminal
到达前失败时，该独立 cleanup 会过早 cancel/remove registry，outer lease 随后无法 drain 原始
producer error，最终可能返回 consumer error 或 `unknown Stream`。

本任务补齐普通可执行 consumer 与既有 supervised lease 的接线，不重新设计 stream ABI。

## 2. 冻结身份与边界

| 项 | 值 |
| --- | --- |
| baseline commit | `5c0f8222972e4612224e0660e88e6054874ddd03` |
| baseline tree | `cf98566873d974a63a9759a2856ecc28efbde5a4` |
| branch | `codex/p5-f445h-i7-s-stream-cleanup` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-s-stream-cleanup` |
| integration owner | `/root/phase05_integration_steward` |

允许写入：

- `runtime/eval/src/{env,eval_context,program_execution,program_stream}.rs`；
- `runtime/eval/src/program_stream/**` 的直接测试；
- 本任务与结果文档。

禁止写入：

- public Stream ABI、native provider trait/API、Host registry；
- Exception、`runtime_ops`、native Json、compiler、Internals；
- stable/live instance、MongoDB、网络或共享配置。

如果修复必须修改上述公开/跨层接口，应停止并上报，不自行扩张。

## 3. 完成态

1. prepared producer 传给普通可执行 consumer 时，只有与该参数精确对应的 stream 才继承
   既有 supervised child；普通函数消费其它 stream 仍使用独立 cleanup。
2. 监督信息沿普通函数、assembly 函数和 const 调用的 Eval 环境传递；不放入全局状态，也不延长
   activation、interpreter 或 registry 的整体生命周期。
3. consumer error、return、drop 或外层 cancel 只向既有 lease 提交 cleanup obligation；
   唯一 outer owner 负责 drain/finalize。
4. producer `emit` 后抛出本地名义类型时，返回原始可 catch 的 typed producer error；
   producer 正常 `End` 时保留 consumer error。
5. producer error 由 consumer 直接观察、自然 `End`、嵌套 helper 转发、外层 cancel/drop 均
   exact cleanup，不残留 registry，也不产生 late result。
6. 不新增第二套 registry/lifecycle state machine，不使用字符串判断 terminal。

## 4. 验证

```bash
cargo fmt --all -- --check
cargo test --locked -p skiff-runtime-eval program_stream -- --nocapture
cargo test --locked -p skiff-runtime-eval
cargo check --locked -p skiff-runtime-eval
git diff --check
```

测试必须是纯 Eval、确定性、无 MongoDB/网络；聚焦 selector 必须非零。

## 5. 交接

任务、实现、结果分别提交。leaf 不 merge、不删除自身 worktree/branch、不 push；完成后把
commit/tree、验证证据和实际写集交给 `/root/phase05_integration_steward`。
