# P5-F445H-E4R5A combined RED authoring

状态：Ready。R5 的 tests-only authoring 前置；在 R1-only production base 上建立可执行组合
RED，供 R2/R3/R4 合流后的新验收 Agent使用。本节点不作最终 verdict。

## 直接父节点与精确代码状态

- `P5-F445H-E4R1-evaluator-spine-actual-pending-checkpoint-result.md`
- `P5-F445H-E4R0-evaluator-closure-execution-preflight-result.md`

production base 为 R1 implementation
`b1faea534654c2ee2109f444a6cad6b1168b8445`。此时：

- R1 actual-Pending/checkpoint应 GREEN；
- timeout和concurrent child仍稳定 fail closed；
- activation仍保留旧 pre-suspend；
- stream current-scope/cleanup仍未闭合。

本节点必须在这个精确语义状态上证明 combined selector能够编译、非零运行，并因上述 R2/R3/R4
缺口产生真实预期 RED。不能等叶子合流后才写“永远为绿”的测试。

## 唯一写集

- 新增 `runtime/eval/tests/f445h_e4r_combined.rs`
- 新增 `P5-F445H-E4R5A-combined-red-authoring-result.md`

不得修改 production、现有 tests/fixtures、Cargo/manifest/lockfile、R1S/R2/R3/R4文件或其它
文档。

## 测试合同

selector：

```text
f445h_e4r_combined
```

至少 **5 个实际 Rust 测试函数**，通过 public/真实 linked evaluator入口组合验证，不直接调用
private child helper：

1. R1 actual-Pending Ready/Pending与checkpoint组合保持可运行；
2. timeout statement/expression/catch至少一条在 R1 base真实 RED；
3. concurrent statement/value/Actor至少一条在 R1 base真实 RED；
4. activation Ready/Pending或 serverStream至少一条暴露旧 pre-suspend RED；
5. stream current child scope与非-End cleanup至少一条真实 RED；
6. 可用一条跨表面测试确认 timeout/concurrent/stream终结后 parent scope/Actor frame不泄漏。

每个 RED必须来自 production入口的错误行为或冻结 fail-closed diagnostic，不能用
`assert!(false)`、缺 fixture、compile error、ignored test或直接搜索文本制造。R1 已完成部分应
GREEN，防止 selector只证明“所有东西都坏”。

测试应复用现有 public test-support/linked artifact构造；不得复制 E1/E2/E3/O1–O6状态机。
如果完成至少5条需要修改现有 private fixture或 production visibility，按停止条件报告，不得
越界。

## RED 验证

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5a-combined-red/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5a-combined-red/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5a-combined-red/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5a-combined-red/build/cargo-target \
  cargo fmt --check
git diff --check
```

预期 execution整体失败，但必须记录：

- listing 至少5个非零测试；
- R1已有表面哪些测试通过；
- R2/R3/R4每类至少一个精确失败及其 production原因；
- 失败不是 panic fixture、编译失败或环境问题；
- check/fmt/diff通过。

本节点不运行完整 eval，不修 production，不把 RED 当最终 FAIL。不得运行 stable、live、
network或 MongoDB。

## 停止条件

出现任一情况返回 `TASK_SCOPE_EXPANDED` 或 `TASK_NOT_EXECUTABLE`：

- public真实 evaluator入口不足，必须改 production visibility或现有 private fixture；
- R1-only base没有按父结果冻结，无法区分预期 RED owner；
- 一个测试必须同时发明新的公共 seam或复制 production状态机；
- 一次有界探查后仍不能形成至少5个可执行测试。

不得派子 Agent。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r5a-combined-red
branch   codex/p5-f445h-e4r5a-combined-red
```

先提交 tests，再单独提交 RED result；返回两个 commit、listing、逐 owner失败证据和 clean
worktree。不得 merge、rebase或 push。

风险：高。测试作者不得担任后续 R5B 独立验收；R1 root或任一叶子公共入口变化可能要求由新的
测试修正节点调整，而不是由 R5B顺手修测试。
