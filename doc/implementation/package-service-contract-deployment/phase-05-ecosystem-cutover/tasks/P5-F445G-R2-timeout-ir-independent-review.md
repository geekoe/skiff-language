# P5-F445G-R2 Timeout IR independent review

状态：Ready。只读独立验收，不修改 implementation。

## 直接父节点

- `P5-F445G-timeout-artifact-lowering-link-checkpoint.md`

审计固定 implementation：

`dee2d0b5d67df9a6f3358d68ee835c7695680e21`

## 审计目标

逐项核对父任务目标与实现，不以测试通过替代代码审查：

1. statement/value timeout、`ValueBlock`、compiled concurrent plan 的持久 shape 是否唯一、字段完整；
2. lowering 是否只消费 `PackageSourceModel::execution_semantics()`，没有重新从 AST 推导 lane
   dependencies/kind/duration；
3. checked duration、source site、lane order、前向依赖、tail shape/closure、unknown/corrupt plan 是否
   fail closed；
4. linked-program/linker 是否只转换和校验，不重建 source semantics；
5. File IR v9/v7/v2 与 identity prefix/preimage/golden 是否原子一致，PackageArtifact、
   ServiceContract、RuntimeAssembly 顶层 schema 未被无关改变；
6. timeout 未新增 public callable，`maySuspend` 仍来自 body/call graph；
7. 新代码是否存在重复职责、过长新模块、明显可分离的复制实现；
8. tests 是否覆盖父任务要求的 T02–T04/T17 本层，且没有只测构造器而绕开真实 lowering/link。

重点查看 `dee2d0b5^..dee2d0b5` 全部 37 个文件；同时反搜旧 File IR 版本并区分 deliberate
rejection 与漏迁 consumer。Router 两个已知漏迁点由 F445G-R1 处理，不重复报为新 finding。

## 输出

只新增并提交：

`P5-F445G-R2-timeout-ir-independent-review-result.md`

result 必须给出：

- `PASS`、`PASS_WITH_HANDOFF` 或 `FAIL`；
- 每个 finding 的严重度、文件/位置、可复现证据和最小修复边界；
- 若无 finding，也要逐项说明审计证据；
- 区分 I3 regression、pre-existing fixture debt、Router child handoff。

不得修改 implementation、测试、golden 或其它文档。不得派子 Agent、merge/rebase/push、
stable/live/network。最终 clean。

## worktree

`/Users/geek/workspace/skiff-p5-f445g-r2-ir-review`

branch：

`codex/p5-f445g-r2-ir-review`

base：`dee2d0b5d67df9a6f3358d68ee835c7695680e21`，再 cherry-pick 本任务文档。
