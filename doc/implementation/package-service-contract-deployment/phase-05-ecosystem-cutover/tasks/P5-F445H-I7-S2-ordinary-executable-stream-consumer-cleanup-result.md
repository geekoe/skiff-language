# P5-F445H-I7-S2 普通可执行 stream consumer 监督与清理结果

状态：

```text
PASS
S2_COMPLETE = YES
BLOCKING_ISSUES = 0
PUBLIC_STREAM_ABI_CHANGED = NO
```

普通 Skiff 可执行函数现在会继承 prepared producer 已有的 supervised consumption child。
它内部的 `for in` 不再以独立 cleanup 提前删除同一 registry；唯一 outer lease 保留 drain 和
finalize 所有权，因此 producer 的 typed terminal、consumer terminal 与外层取消按既有
P4-F14R 规则收敛。

## 1. 执行身份

| 项 | 值 |
| --- | --- |
| baseline commit/tree | `5c0f8222972e4612224e0660e88e6054874ddd03` / `cf98566873d974a63a9759a2856ecc28efbde5a4` |
| task commit/tree | `84d76a96da867b62b8624cd9456dd4ba8d3f0eeb` / `9eab73d364741f1428992559cc6df2ea5fc48a8e` |
| implementation commit/tree | `ebb973225cedda346e3cc4466d58e46c71def165` / `3702b760109c64e1422b754797290da689cd9ab3` |
| branch | `codex/p5-f445h-i7-s-stream-cleanup` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-s-stream-cleanup` |
| integration owner | `/root/phase05_integration_steward` |

最终 result commit/tree 由 Git handoff 报告；result 文档不自引用自己的 commit。

## 2. 实际写集

实现：

```text
runtime/eval/src/env.rs
runtime/eval/src/eval_context.rs
runtime/eval/src/program_execution.rs
runtime/eval/src/program_stream.rs
runtime/eval/src/program_stream/supervised_executable_tests.rs
```

文档：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-S2-ordinary-executable-stream-consumer-cleanup.md
  P5-F445H-I7-S2-ordinary-executable-stream-consumer-cleanup-result.md
```

没有修改 capability-context 的既有 state machine、public Stream ABI、native provider
trait/API、Host、compiler、Exception、`runtime_ops`、Internals 或配置。

## 3. 实现收敛

- `Env` 只携带一个“精确 stream value + supervised child”的 Eval 内部投影。目标 stream
  才使用 child cleanup；无关 stream 继续走 standalone cleanup。
- 普通 executable、assembly executable 和 const call 创建子环境时继承该投影，使
  `Stream` 经 helper 转发仍保持同一监督关系。
- prepared producer 调用普通 consumer 前，把它自己已有的 child 注入 consumer 环境；
  不创建新 registry、lease 或全局 owner。
- `for in` 观察到 producer terminal error 时，在既有 child 上记录 typed terminal，然后用
  原有 materialization 路径返回错误。
- outer drain 返回 request-heap-owned producer error 时，普通调用边界用既有 helper 把它
  materialize 回 consumer heap，保留原始名义类型和 catch identity。

该投影随调用环境/future 释放，不持有 Interpreter、activation 或全局 registry owner，不产生
全局生命周期延长。

## 4. RED 与回归矩阵

修复前的直接 Eval RED：

```text
ordinary_executable_stream_consumer_preserves_late_producer_error
unexpected error: slot 98 for identifier is out of bounds
```

这证明 consumer error 先触发 standalone cleanup 后，producer 后续 typed error 已丢失。把
fixture 升级为真实本地名义 `ProducerError` 后，又确认未经调用边界 materialize 时只能得到
request-heap-owned opaque error；最终实现返回可 catch 的原始 `UserException`。

新增确定性覆盖：

| 场景 | 结果 |
| --- | --- |
| 目标外的无关 stream | 不继承监督，保持 standalone |
| producer emit 后 typed throw，consumer 同时失败 | 原始 producer 名义类型获胜 |
| producer 正常 End，consumer 失败 | consumer error 保留 |
| consumer 直接 drain 到 producer error | 原始 typed error |
| producer 自然 End | 正常完成，registry 不悬挂 |
| stream 经嵌套 helper 转发 | 同一监督关系与 typed terminal |
| outer cancel | 返回前 registry 已清理 |
| outer future drop | registry 清理，重复检查无 late result |

测试全部使用内存 Eval runtime，无 MongoDB、网络、stable/live instance。

## 5. 验证证据

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked -p skiff-runtime-eval program_stream -- --nocapture` | PASS，`21/21`，其中新增矩阵全部非零命中 |
| `cargo test --locked -p skiff-runtime-eval` | PASS，unit `414/414`、integration `4/4 + 5/5 + 6/6`、doc `1/1` |
| `cargo check --locked -p skiff-runtime-eval` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

输出只有 baseline 已存在的 compiler/linker dead-code、Eval unreachable pattern 和测试
unused-import warnings；本任务没有新增 warning。

## 6. 验收

| 条款 | 判定 |
| --- | --- |
| 普通 executable consumer 使用既有 supervised child | PASS |
| producer typed error 保持原始 catch identity | PASS |
| normal End 保留 consumer error | PASS |
| nested forwarding 仍受监督 | PASS |
| outer cancel/drop exact cleanup、无 late result | PASS |
| 无关 stream 仍 standalone | PASS |
| 无第二套 registry/state machine或全局 lifetime 扩张 | PASS |
| public ABI/native provider API 零改动 | PASS |

因此：

```text
S2_COMPLETE = YES
BLOCKING_ISSUES = 0
```

本节点只完成普通可执行 stream consumer 的 Eval closure，不宣告整个 I7 完成。
