# P5-F445H-E4R4S activation prepared module closure

状态：Ready。R4 接收时发现的 production 结构修正；完成后才冻结 E4R R5B 验收候选。只移动
R4 已验证逻辑，不改变语义、公共 API或测试。

## 直接父节点与问题

- `P5-F445H-E4R4-current-scope-stream-activation-closure-result.md`
- `P5-F445H-E4R0-evaluator-closure-execution-preflight-result.md`

集成 base 为 `f213534b18c3fc63bbaf6b020421204a8ac4293e`，已合入 R1/R2/R3/R4、两个测试
结构修正和 combined matrix。

R4 implementation `bb64a182e28378854faeb1dc1046dc8c507e1d4c` 的行为证据有效，但
`runtime/eval/src/assembly_execution/async_stream_cancel.rs` 从约 1992 行增长到 2245 行；
其中约 218 行是完整 activation-relative prepared-operation owner：

- test-only activation wait gate；
- `PreparedActivationRelativeServiceCall`；
- `PreparedActivationRelativeServiceOperation`；
- `CompletedActivationRelativeServiceCall`；
- prepare / Ready / wait / finalize；
- fixed service failure import。

这不是 root dispatch或薄接线，应移入单一 child module。R4 task 已明确长 root不得继续堆叠
helper，本节点闭合这一结构遗漏。

## 唯一写集

- `runtime/eval/src/assembly_execution/async_stream_cancel.rs`
- 新增
  `runtime/eval/src/assembly_execution/async_stream_cancel/activation_relative.rs`
- 新增 `P5-F445H-E4R4S-activation-prepared-module-closure-result.md`

不得修改测试、其它 production、Cargo/manifest/lockfile、R4 result或其它文档。

## 结构终态

- `async_stream_cancel.rs` 只新增 child module declaration和必要的 crate-private窄 re-export；
- 上述 activation-relative prepared-operation 类型、impl、helper和 test-only gate整体移入
  `activation_relative.rs`；
- child可以作为 descendant访问 parent private `prepare_provider_unary`、
  `start_provider_stream`及既有 error/channel helper；不要因此扩大 public visibility；
- `EvalContext::prepare_activation_relative_service_call` 和 test-only gate installer的调用方式
  保持不变；
- `eval_context/actual_pending/activation.rs` 无需修改；
- 不复制任何 O3/R4状态机；
- root行数应明显回落，child保持单一 activation prepared-operation责任。

允许为 Rust privacy做最窄 `pub(super)` / `pub(crate)` re-export调整；不得新增公共 API、改变
错误身份、poll顺序、Ready/Pending行为、serverStream同步行为或 service failure import。

## 验证

这是行为保持型重构，不新增测试。使用已有独立矩阵：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r4s-module/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_stream -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r4s-module/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_stream -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r4s-module/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r4s-module/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r4s-module/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r4s-module/build/cargo-target \
  cargo fmt --check
git diff --check
```

必须保持：

- `f445h_e4r_stream` 22 listed、22/22；
- `f445h_e4r_combined` 5 listed、5/5；
- production行为diff仅为等价代码移动/路径调整；
- root/child行数和窄 visibility边界明确。

不运行完整 eval、其它 owner gate、stable、live、network或 MongoDB。

result 记录 implementation/result commit、移动符号清单、拆分前后行数、visibility、两个 selector
实际数量、check/fmt/diff结果和未决问题。

若代码移动需要改变 operation语义、public API、测试、O3/R4 owner或
`eval_context/actual_pending/activation.rs`，返回 `TASK_SCOPE_EXPANDED`；不得顺手重写行为或派
子 Agent。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r4s-module
branch   codex/p5-f445h-e4r4s-module
```

先提交 production结构移动，再单独提交 result；返回两个 commit、22/22与5/5、行数和 clean
worktree。不得 merge、rebase或 push。

风险：中。该提交会改变 production文件布局，因此必须在它合入后才能冻结 R5B验收候选。
