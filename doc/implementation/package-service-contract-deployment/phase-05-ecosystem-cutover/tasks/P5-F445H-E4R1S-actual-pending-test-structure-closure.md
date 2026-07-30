# P5-F445H-E4R1S actual-Pending test structure closure

状态：Ready。R1 接收时发现的 test-only 结构修正；可与 R2/R3/R4 并行，不改变 production、
行为、测试语义或 E4R DAG。

## 直接父节点与问题

- `P5-F445H-E4R1-evaluator-spine-actual-pending-checkpoint-result.md`

R1 implementation `b1faea534654c2ee2109f444a6cad6b1168b8445` 的 23/23 focused evidence有效，
但新增
`actor_concurrent_continuation_tests/evaluator_actual_pending.rs` 达 3303 行，同时拥有 generic
fixture、outbound、Actor、file stream、emit、callback和 canonical wire七组责任。按 workspace
结构约定，不能把该文件作为最终状态保留。

本节点只做机械模块拆分；不得改变任何 test name、fixture语义、断言、poll顺序、gate、production
visibility或测试数量。

## 唯一写集

- `runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending.rs`
- 新增
  `runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/**`
- 新增 `P5-F445H-E4R1S-actual-pending-test-structure-closure-result.md`

不得修改 production、共享 module declaration、
`evaluator_concurrent.rs`、R2/R3/R4文件、Cargo/manifest/lockfile或其它文档。

## 结构目标

parent 文件只保留模块声明和必要的共享可见性接线。按真实责任至少拆出：

- generic evaluator/context/test-poll support；
- outbound interface/legacy service fixture与测试；
- Actor dispatch fixture与测试；
- native/WebSocket/DbQuery；
- file `createFromStream`；
- detached/projected emit；
- callback；
- canonical-wire emit。

允许在上述边界内合并非常小且高度共享的 support，但不得形成另一个超过约 700 行的“大杂烩”
文件；若某个 child仍较大，必须有单一明确 fixture/matrix责任。共享 helper使用最窄
`pub(super)`，不得改 production visibility或复制大段 fixture。

保持全部 23 个 `f445h_e4r_spine` 测试名称、selector inventory、执行结果和 first-poll
Ready/Pending语义逐项不变。建议在拆分前后保存 listing并比较。

## 验证

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r1s-test-structure/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_spine -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r1s-test-structure/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_spine -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r1s-test-structure/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r1s-test-structure/build/cargo-target \
  cargo fmt --check
git diff --check
```

必须为相同 23 listed、23/23 passed。result 记录拆分前后测试名集合比较、每个文件行数、共享
helper边界、验证结果和 production diff为零。

若拆分必须改变 fixture/断言/测试名、production visibility、共享 module declaration，或出现
无法机械解决的 Rust privacy循环，立即返回 `TASK_SCOPE_EXPANDED`；不得顺手修行为。不得派子
Agent。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r1s-test-structure
branch   codex/p5-f445h-e4r1s-test-structure
```

先提交 test-only结构改动，再单独提交 result；返回两个 commit、文件行数、23/23证据和 clean
worktree。不得 merge、rebase或 push。

风险：中。任何测试语义变化都会使 R1 evidence失效；纯文件移动与等价 visibility调整不使
production证据失效。
