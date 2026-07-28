# P5-F445H-E4R4S activation prepared module closure result

状态：`IMPLEMENTATION_COMPLETE / E4R4S_GREEN`。

本节点把 R4 已验证的 activation-relative prepared-operation owner 从
`assembly_execution/async_stream_cancel.rs` 等价移动到唯一 child
`assembly_execution/async_stream_cancel/activation_relative.rs`。调用方式、公共 API、测试和
operation 行为均未改变。

## 1. 输入、提交与写集

| 项 | commit |
| --- | --- |
| integration base | `f213534b18c3fc63bbaf6b020421204a8ac4293e` |
| task start | `ed037416c50d481bf3f38cf3ebdbe5c629c1ca9c` |
| implementation | `e39e242f4190f8988aef1f2e8faadddaf1f57dea` |
| result | 本文独立 result-only commit；精确 hash 由最终交付消息记录，避免 commit 自引用 |

implementation 只修改任务唯一 production 写集：

- `runtime/eval/src/assembly_execution/async_stream_cancel.rs`
- `runtime/eval/src/assembly_execution/async_stream_cancel/activation_relative.rs`

没有修改测试、其它 production、Cargo/manifest/lockfile、R4 result 或其它文档。

## 2. 等价移动与符号清单

完整移动到 child 的符号和责任：

- test-only wait gate：
  - `ACTIVATION_RELATIVE_WAIT_GATE`
  - `ActivationRelativeWaitGateState`
  - `ActivationRelativeWaitGate` 及其 `has_started` / `release`
  - `EvalContext::install_activation_relative_wait_gate_for_test`
  - `wait_activation_relative_gate_for_test`
- prepared-operation：
  - `PreparedActivationRelativeServiceCall`
  - `PreparedActivationRelativeServiceOperation`
  - `CompletedActivationRelativeServiceCall`
  - `EvalContext::prepare_activation_relative_service_call`
  - `PreparedActivationRelativeServiceCall::{ready_result, wait}`
  - `CompletedActivationRelativeServiceCall::finalize`
  - `finish_activation_relative_service_result`

child 继续直接消费 parent 既有 `prepare_provider_unary` 和 `start_provider_stream`，并保持原
checkpoint、internal dispatch record、unsupported stream error 和 fixed service failure import
路径。唯一代码调整是模块 import 与 assembly parent 路径从 `super` 改为 `super::super`；没有改变
poll 顺序、Ready/Pending 分支、serverStream 同步路径、错误身份或 finalize 时机。

`EvalContext::prepare_activation_relative_service_call` 和 test-only gate installer 的调用点均未
修改；`eval_context/actual_pending/activation.rs` 与所有测试文件保持原样。

## 3. 行数与 visibility

| 文件 | 拆分前 | 拆分后 | 变化 |
| --- | ---: | ---: | ---: |
| `async_stream_cancel.rs` | 2245 | 1997 | -248 |
| `async_stream_cancel/activation_relative.rs` | 不存在 | 264 | +264 |

root 只增加 private `mod activation_relative;` 声明并清理已移动 import；无需 re-export。
child 的 module、operation enum、gate state 和 finalize helper 均保持 private。跨 sibling 调用所需
的两个 prepared/completed 类型、test gate 类型及既有方法保持原来的 `pub(crate)`，没有新增
`pub` 项、公共 API 或扩大 parent helper visibility。

## 4. 验证结果

所有 Cargo 命令使用独立 target：

```text
/Users/geek/workspace/skiff-p5-f445h-e4r4s-module/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_stream -- --list` | PASS：`22 tests, 0 benchmarks` |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_stream -- --nocapture` | PASS：`22 passed / 0 failed` |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --list` | PASS：`5 tests, 0 benchmarks` |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --nocapture` | PASS：`5 passed / 0 failed` |
| `cargo check -p skiff-runtime-eval --tests --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

最终输出只有既有 compiler-source/linker warnings、ordinary tests unused import 和
`service_error_channel.rs` unreachable-pattern warning；新增 child 没有 warning。

没有运行完整 eval、其它 owner gate、stable、live、network 或 MongoDB。

## 5. 结论

没有触发 `TASK_SCOPE_EXPANDED`。production 行为 diff 仅为等价代码移动、import 和相对路径调整；
R4 的 22-test matrix 与 combined 5-test matrix 均保持绿色。未决问题：无。
