# P5-F445H-O6R Evaluator DB internal-stop state machines

状态：Ready。D1已经删除公开取消请求，并废止O5D/O6对异常路径cleanup acknowledgement与
exactly-once terminal的过强要求。本节点只在evaluator DB owner内完成actual-Pending接线、正常路径
严格收尾，以及异常内部停止时可由当前写集保证的最小资源收束。完成后解除J1组合验收。

## 直接父节点

- `P5-F445H-D1-internal-execution-stop-semantics-result.md`
- `P5-F445H-O6-evaluator-db-state-machines-result.md`
- `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md`
- `P5-F445H-O5R2-service-db-prepared-runtime-operation-result.md`

production prerequisite为integration commit `c4557271`。

本任务文件完整描述本节点执行需求。父节点提供既有owner、失败前提与actual-Pending seam；需要核对
依据时沿引用链向上读取，不得从顶层设计自行增加DB、Actor、transaction、lease、timeout、错误或
cleanup语义。

## 当前代码事实

- `runtime/eval/src/program_db.rs`同时拥有普通DB、两种transaction source、lease claim/read；
  当前仍直接`.await`全部store operation。
- 普通recoverable operation仍调用借用caller heap的旧runtime入口，尚未消费O5R2的六个
  `prepare_*_runtime` one-shot wait/finalizer。
- E3的`ActorExecutionFrame::await_if_pending`已经拥有唯一first-poll逻辑：first `Ready`不切换
  segment，真实`Pending`才提交/释放/恢复Actor execution；本节点只能调用，不能复制或修改。
- `DbQuery`只递归求值query并在caller heap物化值，不执行DB I/O；它本身不是挂起边界。
- Transaction store可由clone后放进`async move`，形成不借caller heap/env的begin/commit/abort
  future。异常drop无法等待abort，D1已经允许使用driver/session fallback，不再需要新cleanup owner。
- Lease renew当前由裸`tokio::spawn`启动；outer future drop时`JoinHandle`只会detach，renew会继续持有
  store/hold。这是本节点必须修复的production缺陷。
- `program_db.rs`约793行，`db_eval.rs`约1308行；新增operation runner、transaction、lease与fixture
  必须进入职责明确的child module，root只保留入口和薄转发。

上游失败遮挡关系：

```text
普通operation仍借caller heap
  -> 无法安全交给E3 actual-Pending
  -> Actor DB Pending路径无法验收

裸lease renew task
  -> outer internal stop后task detach
  -> TTL无法生效，因为健康续租仍在继续
```

## 生产目标

### 1. 唯一DB wait runner

在`program_db` owner内增加一个窄helper：

1. wait future不得借用caller `RequestHeap`、`Env`、`DbIrEvaluator`或mutable
   `ProgramExecutionContext`；
2. 当前context有Actor frame时只调用既有
   `ActorExecutionFrame::await_if_pending`；无Actor frame时等待同一个future；
3. operation只构造并启动一次，first `Pending`后不得drop再重建；
4. wait完成并恢复Actor segment后，才在caller segment执行decode、finalizer、binding import或
   flow处理；
5. helper不得复制scheduler acquire/release、field codec、identity fence或first-poll逻辑。

允许future拥有clone后的store、owned command和其它不含caller heap handle的数据。

### 2. 普通DB operation

`eval_program_db_operation_with_context`按以下顺序执行：

```text
递归求值并形成DbCommand
  -> 同步prepare caller-heap-free operation
  -> 通过唯一runner等待
  -> 恢复后向caller heap finalize
```

- raw/wire operation的type、selector/query/order/projection、document/change等输入在wait前全部owned；
- raw wait返回owned `DbDocument`、page、count/bool或provider error，decode只在wait后执行；
- recoverable find/create/update/replace必须消费O5R2六个prepared入口及其唯一finalizer；
- 保持现有校验顺序、`limit: 0`、required-not-found、null、result plan、boundary use和recoverable
  context；
- first `Ready`与真实`Pending`都只启动一次；provider/finalizer error不得重放副作用；
- finalizer失败继续使用O5R2 checkpoint规则，不在caller heap留下半物化结果；
- 不保留新旧两套heap-borrowingruntime状态机。

### 3. `DbQuery`

`eval_program_db_query_value`继续直接调用`DbIrEvaluator::eval_query_value`。不把整个query包装进
runner，不预先释放Actor segment；query内部真正外部wait由对应嵌套expression自己的owner处理。

### 4. Transaction

legacy `db.transaction(...)`与显式`DbTransactionIr`共用一个evaluator lifecycle core：

- begin、commit和abort分别构造成clone-store、caller-heap-free的one-shot future，并各自通过唯一
  actual-Pending runner；
- begin失败不abort；body/result/非法flow失败时严格等待abort，再保留原错误并执行既有checkpoint
  truncate；normal success严格等待commit；
- commit返回错误时保持现有顺序：严格等待一次abort，保留commit error，不重试commit；
- normal commit后不abort；terminal选择必须由显式phase表达，不能由多个松散bool产生double terminal；
- outer execution在commit选择前被内部停止/drop时，不得commit、不得裸spawn async abort、不得阻塞
  `Drop`；释放当前future/store/context并依赖service-db session/driver fallback是D1允许的异常收束；
- commit future一旦开始poll，后续内部停止/drop只能drop同一个waiter并隔离晚到结果，不得改选abort、
  重建commit或声称rollback；provider outcome可以完成或unknown；
- checkpoint只保持现有“截断新增allocation”语义，不声称撤销既有heap节点的原地mutation。

这里的“严格等待”只约束正常成功和普通错误路径；若该等待本身又遭遇外层内部停止，仍按异常规则drop并
fallback。

### 5. Lease claim

`eval_program_db_lease_claim`按以下阶段执行：

```text
key prepare
  -> claim actual-Pending wait
  -> binding import + renew owner
  -> body evaluator
  -> stop/join renew owner
  -> lease_lost actual-Pending wait
  -> release actual-Pending wait
  -> visible terminal
```

- claim `None`返回`false`，不启动renew、不release；
- claim成功后才向caller heap/env导入binding；
- renew owner只拥有clone store、hold、period和内部stop carrier，不借caller heap/env；
- 增加RAII renew owner：其`Drop`必须同步调用`JoinHandle::abort()`，因此outer future任意阶段drop都
  不会detach续租task；不得在`Drop`中await或spawn cleanup；
- 正常成功、业务错误和非法flow路径必须停止并join renew task，再读取lease-lost并严格等待release；
- renew失败继续触发现有request内部停止/lease-lost状态；lease-lost、release error与body error的
  可见优先级保持当前行为；
- terminal后late renew不得重新改变状态或启动第二次release；
- 异常内部停止/drop只保证RAII owner停止续租。Release可以未开始或其waiter可被drop，平台依赖现有
  lease TTL回收；不得增加detached release task、cleanup acknowledgement或exactly-once断言。

### 6. Lease read

- 递归求值key并在caller segment编码为owned `DbKey`；
- `read_lease`使用caller-heap-free actual-Pending wait；
- 恢复后才向caller heap decode；`None`仍返回`Null`；
- Ready/Pending各启动一次，drop/error不重试。

## 测试与完成标准

先增加真实RED，再实现。测试至少穿过真实`eval_program_db_*`入口和真实
`ActorExecutionFrame`/store fixture，不能只测抽象helper。至少覆盖：

1. 纯`DbQuery` Ready不commit/release/reacquire；
2. raw ordinary operation的first-Ready与真实Pending，副作用各一次，Pending只切一个segment；
3. recoverable ordinary operation的first-Ready与真实Pending，证明消费O5R2 wait/finalizer；
4. Pending期间caller heap可独立使用，finalize前不变、恢复后结果可见；
5. provider error first-Ready与Pending-after-error均不重建future；
6. transaction begin/body DB operation/commit/abort各自的Ready/Pending与segment计数；
7. begin failure不abort、normal commit不abort、body/flow/commit error普通路径只abort一次；
8. commit真实Pending后drop不重建commit、不调用fallback abort，也不物化late result；
9. transaction body期间drop不commit；测试不得要求async abort acknowledgement；
10. 两种transaction source共用lifecycle core；
11. lease claim `None`、Ready与Pending，binding在claim成功后才导入；
12. lease正常成功/业务错误/非法flow停止并join renew，读取lease-lost并等待release；
13. lease body期间drop会触发RAII abort renew task，之后renew计数不再增长；允许没有release，并把TTL
    fallback作为预期；
14. release真实Pending期间drop不重建release；
15. lease read Ready/Pending/None/decode error；
16. Actor first `Ready`不切segment，真实`Pending`只切一次；E3 identity/replacement fail-closed保持。

Fake store、gate future、poll/drop计数器、Actor fixture放入窄child test module。不得继续扩大
`program_db.rs`或`db_eval.rs`中的内联测试块。

开发Agent只拥有以下验证：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r-eval-db/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r-eval-db/build/cargo-target \
  cargo test -p skiff-runtime-eval db_actor -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r-eval-db/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r-eval-db/build/cargo-target \
  cargo check -p skiff-runtime-eval -p skiff-runtime-service-db \
    -p skiff-runtime-capability-context --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r-eval-db/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录每个selector的实际测试数；零测试不算证据。不得连接MongoDB、运行stable/live、访问网络或运行
阶段完整gate。

反向搜索必须证明：

- 普通recoverable DB不再调用六条借heap的`*_runtime(..., heap).await`；
- transaction/claim没有把借caller heap/env的整段future交给runner；
- 不存在pre-suspend、`yield_now`、unsafe heap alias、heap mutex、future restart、detached cleanup
  task或复制的Actor scheduler；
- lease renew `JoinHandle`在所有drop路径都由RAII owner abort；
- `DbQuery`没有新增外部wait wrapper。

## 写入范围与停止条件

只允许：

- `runtime/eval/src/program_db.rs`
- `runtime/eval/src/program_db/**`
- `runtime/eval/src/db_eval.rs`
- `runtime/eval/src/db_eval/**`
- 本result

不得修改capability-context、service-db、Host/request、E1/E2/E3、其它evaluator call site、compiler、
artifact、Router、manifest或lockfile。

若实现需要写集外production owner、公共API变化、request级cleanup supervisor、detached async
abort/release、复制E3状态机或新的timeout配置，必须在一次有界探查后停止并返回
`TASK_SCOPE_EXPANDED`。若五分钟内仍不能形成写集内的第一处实际代码修改，返回
`TASK_NOT_EXECUTABLE`及精确缺口，不静默研究。

风险：高（Actor actual-Pending、transaction、lease并发生命周期）。本节点完成后只是实现检查点，
不是稳定候选；J1将作为独立组合验收owner检查DB与其它prepared operation的统一路径。

## Worktree与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6r-eval-db
branch   codex/p5-f445h-o6r-eval-db
```

先提交implementation，再单独提交
`P5-F445H-O6R-evaluator-db-internal-stop-state-machines-result.md`。最终worktree clean；不得
merge、rebase或push。

这是一次性有界开发会话。当前production与测试写入集中在同一owner，没有适合独立交付的并行子块；
不要派子Agent。遇到范围扩张或仍有多个不明确问题时按工作流停止并如实上报。

