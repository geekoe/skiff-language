# P5-F445H-O6R1 DB/Actor fixture owner preflight

状态：Ready。O6R production已经形成实现检查点，但缺少真实
`eval_program_db_*` + `ActorExecutionFrame` + fake store的Ready/Pending/error/drop验收矩阵。本节点
只冻结最小可执行测试owner与任务拆分，不修改production或tests。

## 直接父节点

- `P5-F445H-O6R-evaluator-db-internal-stop-state-machines-result.md`

production prerequisite为integration commit `6ef1bf9f`。

父result已经说明实现内容和证据缺口；需要核对actual-Pending与Actor依据时，只沿其直接引用向上读取。
不得从顶层设计增加新语义。

## 唯一问题

在不修改production API、capability-context/service-db production或E3 scheduler的前提下，怎样用最小
test-only写集覆盖：

1. raw与O5R2 prepared ordinary operation的first-Ready/真实Pending/error/drop；
2. 两种transaction source的begin/body/commit/abort phase；
3. lease claim/read/lost/release与renew owner drop；
4. 至少一条真实Actor execution证明Ready不切segment、Pending只切一次且恢复后结果物化；
5. operation start/poll/terminal/drop计数和禁止future restart。

## 有界审计范围

只读：

- `runtime/eval/src/program_db.rs`与`program_db/**`
- `runtime/eval/src/actor_executor.rs`、`actor_executor/**`
- `runtime/eval/src/program_execution.rs`
- `runtime/eval/src/assembly_execution/ordinary/test_runtime.rs`
- `runtime/capability-context/src/db.rs`及现有prepared fake tests
- `runtime/driver/eval/tests/program_execution.rs`及其module owner
- 父result点名的直接上游test fixture

不泛化搜索其它runtime领域，不运行测试，不访问network/stable/live/Mongo。

## 必须交付的冻结结论

Result必须给出：

- 推荐测试落点和精确module/file写集；
- 是否能复用现有Actor fixture与test runtime，精确说明可见性和依赖方向；
- fake `DbCapabilityContextApi` / `DbCapabilityStoreApi`的最小形状，哪些trait method需要真实实现、
  哪些只需fail-fast stub；
- 形成真实`eval_program_db_*`输入所需的最小linked IR/executable fixture owner；
- 是否需要一个先行共享fixture checkpoint；若需要，列出其稳定接口和后续可并行测试节点；
- 每个测试节点的真实RED、非零selector、完成标准和写入不重叠方式；
- 能否完整留在test-only写集。若不能，返回`TASK_SCOPE_EXPANDED`并指出唯一缺少的production/test-support
  seam，不得自行设计或实现它；
- 明确哪些O6R task条款过大或不可测试，不能用“全矩阵”笼统带过。

推荐优先寻找“一个共享fixture checkpoint + ordinary/transaction/lease非重叠child tests”的短扇出；
只有代码事实证明共享成本更高时才建议单任务。

## 写入与交付

只允许本result：

- `P5-F445H-O6R1-db-actor-fixture-owner-preflight-result.md`

禁止修改production、tests、Cargo、manifest、lockfile或父任务/result。不得派子Agent。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6r1-fixture-preflight
branch   codex/p5-f445h-o6r1-fixture-preflight
```

单独提交result，worktree clean；不得merge、rebase或push。若一次有界审计后仍有多个会改变拆分方向的
未知量，返回`TASK_NOT_EXECUTABLE`，不要继续扩大搜索。

