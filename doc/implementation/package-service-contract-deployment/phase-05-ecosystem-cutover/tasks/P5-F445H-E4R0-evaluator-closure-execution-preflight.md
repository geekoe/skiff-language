# P5-F445H-E4R0 evaluator closure execution preflight

状态：Ready。J1 已确认 O1–O6 owner prerequisite 闭合，但原 E4 同时跨 timeout/catch、concurrent、
actual-Pending call-site、checkpoint 与 stream cleanup，且 `eval_context.rs` 已 2159 行。
本节点只读冻结最小可执行 E4R DAG、写集与测试 owner；不实现代码。

## 直接父节点

- `P5-F445H-J1-prepared-operation-combined-review-result.md`
- `P5-F445H-E4-evaluator-catch-stream-closure-result.md`
- `P5-F445H-E4-evaluator-catch-stream-closure.md`
- `P5-F445H-E23-concurrency-branches-combined-result.md`

父节点沿 E1/E2/E3/E3R/O1–O6 追溯到唯一权威设计。当前 integration checkpoint 为
`99acfd13`；E4R 不需要用户重新决定 actual-Pending、无 yield、Ready不释放或 internal stop语义。

## 唯一写集

- `P5-F445H-E4R0-evaluator-closure-execution-preflight-result.md`

不得修改 production、tests、既有 task/result、Cargo、manifest 或 lockfile。不得运行测试、编译、
stable、live、network 或 MongoDB；允许只读 `rg`、文件阅读、`git` 查询和行数/模块图检查。

## 必须回答的具体问题

### 1. 最小 DAG

把原 E4 完成标准拆成尽量少、但每个都可在授权写集内独立实现和聚焦验证的节点。至少判断：

- timeout statement/expression + owner materialization + ordinary catch是否必须同节点；
- concurrent statement/value + Actor bridge是否必须同节点；
- owner-aware checkpoint覆盖是否应跟 timeout/concurrent core还是单独串行；
-九处 actual-Pending call-site cutover是否可与 timeout/concurrent并行，还是必须按
  `eval_context.rs` root串行；
- stream current scope/cleanup能否在不改 `eval_context.rs` 的独立写集中并行；
-最终真实四 arm + stream + actual-Pending组合验证应由一个 integration/acceptance节点拥有。

不得为了增加 Agent 数量机械拆分同一条串行链。若建议 shared structural checkpoint，必须说明它
解除哪些并行节点，以及 checkpoint 自身怎样非零验证。

### 2. 精确写集与结构

列出每个建议节点：

- production文件；
-新 child module；
- test文件；
-明确禁止的并行 surface；
-是否会修改 `runtime/eval/src/lib.rs` 或 module exports。

重点检查 Rust privacy/lifetime：

- child module 是否能对 `EvalContext` 定义 `pub(super)` helper并访问现有 fields；
- root match arm是否必须同时修改；
- `async_recursion` / Send future 是否限制 helper返回类型；
- E2 lane executor与 E3 concurrent bridge的真实消费位置；
- O3 `async_stream_cancel.rs` 与 E4 stream修改是否能保持单一 owner；
- `program_invocation.rs` / `program_stream.rs` 的现有 child test结构是否可复用。

若两个节点不可避免编辑同一 root/import区，标记串行依赖；不要把 merge conflict留给主 Agent猜。

### 3. 真实入口与测试 owner

定位当前已有 fixture/helper与缺口，给每个节点冻结：

-真正 RED；
-至少一个具名非零 selector；
-预计最小测试函数数；
-必须穿过的 production入口；
-禁止只测 leaf helper的降级方式；
-哪些昂贵 gate只由最终 combined owner运行。

测试覆盖必须完整映射原 E4 的 T05–T12：

- timeout normal/value/parent恢复、local owner materialization/catch、nested deadline、
  inherited deadline、cancel优先；
- scripted clock纯 CPU loop/chunk；
- concurrent dependency/tail/source-order、Actor Ready/Pending重叠、winner/late result/outer restore；
- actual-Pending native/service/interface/Actor/callback Ready与Pending，WebSocket send/serverStream/
  DbQuery同步例外；
- stream End与 break/return/error/timeout/cancel/drop cleanup，current child scope；
-既有 ValueBlock、ordinary catch/rethrow、Actor continuation与stream invocation回归。

必须说明怎样避免重复 O6R13/J1 已拥有的 gate。

### 4. 执行性与停止条件

对每个节点明确：

-直接前置与解除的后续；
-当前代码是否已具备全部 seam；
-从任务启动到首次可安全修改是否能在五分钟内完成；
-若仍需 O1–O6 core、E1/E2/E3 public API、host/I6或公共契约修改，应返回
  `TASK_NOT_EXECUTABLE` 或 `TASK_SCOPE_EXPANDED`，列精确证据；
-若存在会改变语言/公共语义的选择，列为用户决策；没有则明确无。

## 结果格式

返回以下之一：

- `READY_FOR_E4R_DAG`：给出节点表、依赖图、写集、selector、测试数门槛、集成顺序、风险与停止条件；
- `TASK_SCOPE_EXPANDED`：列新 owner与最小前置；
- `DECISION_REQUIRED`：只列真正改变语言/公共语义的最小问题。

不要写实现建议的大段伪代码；结果要足够让主 Agent直接派发叶子任务，不依赖聊天摘要。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r0-preflight
branch   codex/p5-f445h-e4r0-preflight
```

只提交 result 文档；worktree clean。不得 merge/rebase/push，不得派子 Agent。
