# P5-F445H-E3R Heap-borrowing actual-Pending preflight

状态：Ready。E4 停止后新增的有界只读 owner 探查；只确定安全实现方向与最小 DAG，不修改
production或 tests。

## 直接父节点

- `P5-F445H-E3-actor-concurrent-continuation-bridge-result.md`
- `P5-F445H-E4-evaluator-catch-stream-closure-result.md`

本任务文件完整冻结语义：

- Actor同步 segment只有在外部 future第一次真实返回 `Pending` 时才提交并释放；
- 第一次 poll为 `Ready` 时不提交、不释放、不 reacquire；
- runtime没有显式 `yield`，也不按 `maySuspend`或调用种类预释放；
- 第一次 poll可能已经产生同步 heap/Actor mutation或外部副作用，不能 clone旧 heap、drop并重启
  future、unsafe别名或退回 pre-suspend；
- pending/cancel/drop/resume必须保持 exactly-once commit、scheduler guard、identity fence与
  field import合同。

不得从更高层设计文档扩大需求。

## 固定输入与问题

Skiff integration `d9b504ee`。E4 已证明现有
`ActorExecutionFrame::await_if_pending(&mut RequestHeap, ..., Future)` 不能包装本身借用同一
heap或整个 `&mut EvalContext` 的 future。

需要回答的不是“怎样让 borrow checker通过”，而是：在第一次 poll结束且 future仍然存活时，
哪个 owner能够安全取得该同步 segment的真实最终 Actor state并提交。

## 必答问题

### 1. Actor field与 heap别名事实

从 production IR/eval/model代码证明：

- `ActorSelfField` read/write如何把 `RuntimeValue`、heap handle和
  `ActorInstanceExecutionLease` 的 field/heap联系起来；
- 读到 array/map/object等 heap-backed field后，用户代码能否经 nested field/index mutation在
  不再次调用 `ActorExecutionFrame::write_field` 的情况下改变 Actor可达状态；
- `resume`、`commit_execution`、field codec和 heap clone各自在哪一份 heap上工作；
- synchronous first poll若修改 Actor field alias，现有 frame之外是否有任何可靠 mutation hook。

必须构造最小代码路径或现有 test证据。若 alias mutation合法，明确判定“只在显式 field write
刷新 detached snapshot”是否会丢状态；若不合法，给 compiler/runtime的完整禁止证明，不能只靠
预期。

### 2. Detached snapshot方向

审查 E4 result建议的 detached canonical field snapshot，逐项回答：

- snapshot如何在所有可达 mutation后保持最新；
- snapshot存 wire、独立 heap还是 field value，各自怎样保留 alias/cycle/失败原子性；
-第一次 poll为 Pending时如何在不读 live caller heap的情况下提交真实终态；
- Ready、Pending、pending future error/drop/cancel、resume失败、instance replacement与 nested
  concurrent bridge如何收束；
- 是否需要修改 Actor field可观察语义、RequestHeap公共模型或引入 pervasive mutation hook。

只有能对 alias case给出可实现的闭环，才能推荐该方向。

### 3. Operation分阶段方向

枚举 `eval_context.rs` 所有现有 `suspend_actor_segment` / `resume_actor_segment` 对及相关 stream
wait，按以下类别建立表：

- future完全不借用 caller heap/env，可直接消费现有 poll-once seam；
-同步 prepare可在当前 segment完成，wait future可自然拥有全部参数且不借用 heap，resume后再
  finalize；
-当前 async fn把 prepare/wait/finalize混在一起但可以在单一 owner内安全拆开；
-事务、callback或递归 evaluator等不能简单分阶段，需要专门状态机/不同 owner；
-其实不是外部 suspension point，不应释放 Actor segment。

至少覆盖 DB operation/query/transaction/lease、activation-relative service、legacy outbound
service、remote/callback interface、Actor dispatch、native time/file/HTTP/WebSocket/stream、
connection send/request和已有 stream next。

对可分阶段调用记录：

```text
prepare 的 heap/env mutation
owned wait state/future
finalize 的 heap/env import
cancel/drop owner
最小 production/test写集
```

判断是否可以建立一个统一 crate-private `PreparedExternalOperation` / pending guard协议，还是必须
按 owner拆分；不得为了表面统一隐藏不同 transaction语义。

### 4. 其它安全方向

明确审查并接受或排除：

- closure/HRTB形式的 `await_if_pending`；
- move heap into future；
- `RefCell`/Mutex/interior mutability；
- custom Future在 Pending时归还 heap；
- poll hook/callback；
- operation返回 `Ready`或 `Pending { owned_wait, finalize }` 的显式状态机。

判定依据必须同时包括 Rust所有权与 Actor语义，不能只说“能编译”。

### 5. 最小 DAG与修正合同

输出推荐方向后，给出：

- 精确 prerequisite节点、互斥写集和 join顺序；
- 哪些属于 E3 correction，哪些属于 operation owner refactor，哪些仍留给 E4；
- 每个节点真实 RED、focused GREEN与停止规则；
- 是否需要修改原 E4任务写集/重新发 E4；
- 独立组合审查点；
- 是否存在需要用户决定的语言/产品语义。

若需要跨多个 owner，不得假装是一个“小 E3 correction”；应按真实职责拆分，但不要为了填槽拆
没有独立验收意义的节点。

## 输出与边界

只允许新增
`P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md`。结果必须是：

- `READY_FOR_CORRECTION_DAG`
- `TASK_SCOPE_EXPANDED`
- `DECISION_REQUIRED`

并包含精确文件/函数证据、call-site分类表、方向比较、推荐 DAG、测试矩阵和用户决策结论。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e3r-preflight
branch   codex/p5-f445h-e3r-preflight
```

不得修改 production/tests/父文档/Cargo manifest/lockfile，不运行 stable/live/network，不派子
Agent，不 merge/rebase/push。探查后仍有多个不明确问题时必须如实结束，不得自行假设。
