# P5-F445H-E4R5AS combined test structure closure result

状态：`PASS`。

test-only 结构改动已在 implementation commit
`2c0eab53674af3af04fcba1ee28ed8f63ef671df` 完成。原 2015 行
`runtime/eval/tests/f445h_e4r_combined.rs` 现在只保留 2 行窄 module 接线；generic
execution/capability harness、stream probe、Actor executable/harness、activation artifact
compile/hydration 和五个 owner case 均已分离。fixture、linked IR、gate/poll 顺序、timeout、
断言、错误文本和当前 R1/R2/R3 GREEN、R4 两条 RED 均未改变。

本文件位于后续独立 result commit；其实际 hash 随交付回报记录。

## 1. 拆分前后测试名与机械等价

拆分前后函数名集合均为：

- `f445h_e4r_combined_r1_actual_pending_ready_pending_and_checkpoint_stay_runnable`
- `f445h_e4r_combined_r2_timeout_statement_and_expression_execute`
- `f445h_e4r_combined_r3_concurrent_statement_value_and_actor_execute`
- `f445h_e4r_combined_r4_activation_ready_error_keeps_actor_segment`
- `f445h_e4r_combined_r4_stream_observes_child_scope_and_cleans_non_end`

focused listing 前后都为精确 `5 tests, 0 benchmarks`，没有新增、删除或 ignore 测试。
拆分后的 qualified path 增加了
`f445h_e4r_combined::<owner_case>::` 职责模块段，但五个测试函数名逐字不变。

将所有 child 按原文件顺序重组后，忽略本任务唯一需要新增的 `pub(super)`、空白和 rustfmt
可选尾逗号，规范化 SHA-256 前后均为
`00cbd6d93bf77c1e3a496e5046f6025192cea28b680305d73eb0581bb2624c2d`。
该比较覆盖原文件全部内容，包括 capability 实现、状态与 fixture、linked IR、artifact
source、gate、poll 顺序、timeout、断言和错误文本；没有复制 harness、状态机、fixture 或
capability 实现。

## 2. Execution 等价

任务命令在拆分前后均整体按预期 exit `101`，汇总保持：

```text
test result: FAILED. 3 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out
```

逐项结果保持：

| owner | 结果 |
| --- | --- |
| R1 | PASS：actual-Pending、Ready/Pending 和 checkpoint case 成功。 |
| R2 | PASS：timeout statement/expression case 成功。 |
| R3 | PASS：concurrent statement/value/Actor case 成功。 |
| R4 activation | 预期 RED：`R4 expected first-Ready activation failure to retain the Actor segment; R1 pre-suspend let the queued competitor run first` |
| R4 stream | 预期 RED：`R4 expected current child scope to terminate pending next() before cleanup; harness timeout while stream next remained pending; next received 1 cancellation token(s)` |

两条 R4 仍由原 production 原因失败；本任务没有修改 expected result、放宽断言或修复 R4。

## 3. 文件责任与行数

| 文件 | 行数 | 单一责任 |
| --- | ---: | --- |
| `f445h_e4r_combined.rs` | 2 | root integration test 的窄 `#[path] mod` 接线 |
| `mod.rs` | 15 | child module 声明 |
| `imports.rs` | 87 | child 共用的 test-only dependency import |
| `common.rs` | 64 | service identity、source site 与 linked IR primitive builder |
| `stream_support.rs` | 188 | pending stream probe、cancel signal 与 sink |
| `runtime_factory.rs` | 77 | stream runtime factory 与 no-op test effects |
| `execution_control.rs` | 215 | execution scope/control、instruction count 与 config gate |
| `capability_harness.rs` | 395 | file、Actor、WebSocket、effect、HTTP 与 outbound capability adapters |
| `execution_harness.rs` | 117 | program execution context 与 interpreter/runtime assembly |
| `actor_support.rs` | 330 | Actor executable IR、instance activation 与 method executor harness |
| `activation_support.rs` | 287 | activation artifact authoring、runtime assembly、link 与 hydration |
| `poll_support.rs` | 12 | deterministic first-poll probe |
| `r1_case.rs` | 21 | R1 owner case |
| `r2_timeout_case.rs` | 23 | R2 timeout owner case |
| `r3_concurrent_case.rs` | 23 | R3 concurrent owner case |
| `r4_activation_case.rs` | 104 | R4 activation owner case |
| `r4_stream_case.rs` | 100 | R4 stream owner case |

最大 child 为 395 行的 `capability_harness.rs`；没有把约 1750 行 support 整体平移到另一个
单文件。

## 4. 共享边界

- `imports.rs` 只在本 integration-test module tree 内以 `pub(super) use` 提供共享类型。
- common IR builder、stream probe state、runtime factory、execution control、capability
  adapter、execution context、Actor harness、activation instruction 和 first-poll probe只对共同
  parent 使用最窄的 `pub(super)`。
- sibling 不需要的 helper、字段与 adapter 实现仍为 private；没有 `pub(crate)`、production
  visibility 或 production API 变化。
- support 依赖保持单向窄接线；首次 `cargo check --tests` 即通过，没有 Rust privacy 循环。

## 5. 验证

所有 Cargo 命令使用独立 target
`/Users/geek/workspace/skiff-p5-f445h-e4r5as-test-structure/build/cargo-target`，并以 Cargo
offline 模式执行以确保不访问 network。

| 命令 | 拆分前 | 拆分后 |
| --- | --- | --- |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --list` | PASS；5 tests | PASS；5 tests |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --nocapture` | 预期 RED；3 passed / 2 failed | 预期 RED；3 passed / 2 failed |
| `cargo check -p skiff-runtime-eval --tests --locked` | 不需要建立拆分前结构证据 | PASS |
| `cargo fmt --check` | 不需要建立拆分前结构证据 | PASS |
| `git diff --check` / staged diff check | 不适用 | PASS |

输出只有父结果已记录的 compiler/linker dead-code/unused warning、写集外 ordinary test unused
import 和写集外 service error channel unreachable pattern；拆分后的 combined test 没有新增
warning。

## 6. 写集与停止条件

implementation commit 的全部 17 个 changed paths 均为
`runtime/eval/tests/f445h_e4r_combined.rs` 或新增
`runtime/eval/tests/f445h_e4r_combined/**`。对 `runtime/eval/src`、Cargo manifest 与
`Cargo.lock` 的 diff 为零；没有修改其它 test/fixture、R4 文件、production 或其它文档。

未触发 `TASK_SCOPE_EXPANDED`：拆分不需要改变测试语义、production visibility、Cargo 或
existing fixture，也没有 privacy 循环。未访问 stable/live/network，未启动本地服务，未
merge、rebase 或 push。
