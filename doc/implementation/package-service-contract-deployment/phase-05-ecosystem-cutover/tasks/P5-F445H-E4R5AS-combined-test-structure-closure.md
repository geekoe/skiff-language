# P5-F445H-E4R5AS combined test structure closure

状态：Ready。R5A 接收时发现的 test-only 结构修正；与 R4 并行，不改变 production、测试语义、
预期 RED或最终验收角色。

## 直接父节点与问题

- `P5-F445H-E4R5A-combined-red-authoring-result.md`
- `P5-F445H-E4R1S-actual-pending-test-structure-closure-result.md`

R5A tests commit `69bb9c57a6b0ae4174f511626b36307207fbf5ca` 建立了有效的 5-test combined
matrix，但 `runtime/eval/tests/f445h_e4r_combined.rs` 达 2015 行。它同时承载 generic
capability harness、Actor fixture、stream fixture、activation artifact编译和五个 owner case；
不能作为最终结构保留。

本任务 branch base 已合入 R2/R3，因此预期执行状态从 R5A 的 `1 passed / 4 failed` 前进为
`3 passed / 2 failed`：R1/R2/R3 GREEN，R4 activation/stream仍 RED。该变化来自 production
base，不是本结构任务要修复的内容。拆分前后必须在同一 branch base上保持完全相同的 test名称、
断言和 `3/2` 结果。

## 唯一写集

- `runtime/eval/tests/f445h_e4r_combined.rs`
- 新增 `runtime/eval/tests/f445h_e4r_combined/**`
- 新增 `P5-F445H-E4R5AS-combined-test-structure-closure-result.md`

不得修改 production、Cargo/manifest/lockfile、现有其它 tests/fixtures、R4文件或其它文档。

## 结构目标

root integration test只保留 `#[path = ...] mod ...` 或等价模块声明与必要窄接线。至少分离：

- generic execution/capability harness；
- stream probe与stream case support；
- Actor executable/harness；
- activation artifact compile/hydration；
- R1 case；
- R2 timeout case；
- R3 concurrent case；
- R4 activation case；
- R4 stream case。

可以按 Rust privacy和共享程度组合小模块，但不得把约 1750 行 support整体平移成另一个单文件。
每个较大 child必须有单一清楚责任；共享符号使用最窄 `pub(super)` / `pub(crate)` test-only
visibility。不得复制 harness、状态机、fixture或 capability实现。

机械等价要求：

- 五个测试函数名逐字不变；
- linked IR、artifact source、gate、poll顺序、timeout、断言和错误文本不变；
- 不放宽 R4 RED，不修改 expected result；
- 不增加/删除/ignore测试；
- production diff为零。

## 验证

先在拆分前保存 listing、函数名集合和 execution；拆分后比较：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5as-test-structure/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5as-test-structure/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5as-test-structure/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5as-test-structure/build/cargo-target \
  cargo fmt --check
git diff --check
```

预期前后均为 5 listed；当前 integration base预计 R1/R2/R3通过、R4两条以原 production原因失败。
若实际 baseline不是此状态，先记录精确差异并停止，不得把 production变化吞进结构任务。

result 记录拆分前后测试名集合、execution逐项结果、每个文件行数、共享边界、production diff
为零、check/fmt/diff结果。

若拆分需要改变测试语义、production visibility、Cargo、existing fixture，或一次有界尝试仍有
Rust privacy循环，返回 `TASK_SCOPE_EXPANDED`。不得顺手修R4，也不得派子 Agent。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r5as-test-structure
branch   codex/p5-f445h-e4r5as-test-structure
```

先提交 test-only结构改动，再单独提交 result；返回两个 commit、5-test等价证据、当前3/2结果、
文件行数和 clean worktree。不得 merge、rebase或 push。

风险：中。任何断言/fixture语义变化都会使 R5A RED证据失效；纯模块移动和等价 test-only
visibility调整不失效。
