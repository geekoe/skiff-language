# P5-F445H-O6 Evaluator DB actual-Pending state machines

状态：Ready。O5R2 已提供 caller-heap-free 的 recoverable runtime operation；本节点只负责
evaluator DB owner，把普通 DB、transaction、lease claim/read 接入既有 E3 actual-Pending
Actor continuation。完成并验收后才能启动 J1 组合审查。

## 直接父节点

- `P5-F445H-E3-actor-concurrent-continuation-bridge-result.md`
- `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md`
- `P5-F445H-O5R2-service-db-prepared-runtime-operation-result.md`

production prerequisite 为 Skiff integration `69ba325a`。

本任务文件完整描述本节点需求。直接父节点只提供已经实现的 seam、现状证据和唯一上游引用；
不得从更高层设计自行增加 DB、Actor、transaction、lease、timeout 或错误语义。

## 当前检查点

- E3 的 `ActorExecutionFrame::await_if_pending` 已冻结：第一次 poll 为 `Ready` 时不提交、不释放、
  不 reacquire；第一次真实 `Pending` 时才提交当前 caller heap、等待同一个 future并 resume。
- O5R2 的六个 `prepare_*_runtime` 返回 caller-heap-free one-shot wait和 resume 后 finalizer；
  不得回退到旧 heap-borrowing runtime入口。
- `DbIrEvaluator::eval_operation` 仍在当前同步 segment中递归求值 argument/query/body并构造
  `DbCommand`；这些嵌套 expression自己的 suspension由其 evaluator owner负责。
- `DbQuery` 只求值 query expression并在 caller heap物化 query value，没有 DB I/O；它本身不是
  调度让出点。
- `program_db.rs` 仍把普通 DB wait、transaction begin/commit/abort、lease
  claim/renew/read/release直接 `.await`，尚未逐个通过 E3 actual-Pending seam。
- `RequestHeap::rollback_to_checkpoint` 只撤销 checkpoint之后新增的节点与统计，不还原既有
  object/array/map的原地 mutation；结果和测试不得把它描述成完整内存事务。

语言尚未发布，不保留预释放、future重建或历史兼容路径。

## 生产目标

### 1. 唯一 heap-free wait runner

在 `program_db` owner中增加一个窄 helper，统一执行 DB 外部 wait：

1. future不得借用 caller `RequestHeap`、`Env`、`DbIrEvaluator`或 mutable
   `ProgramExecutionContext`；
2. 当前 context有 Actor frame时，只调用既有 `ActorExecutionFrame::await_if_pending`；
3. 无 Actor frame时直接等待同一个 future；
4. helper不得复制 scheduler acquire、lease、field codec、identity fence或 first-poll逻辑；
5. operation副作用只启动一次；第一次 poll为 `Pending`后不得 drop并重建 future；
6. wait返回后才在 caller segment执行 finalizer、wire decode、binding import或 flow处理。

允许 future借用 caller-heap无关的 clone/owned store与 owned command；若借用局部不可变值，必须
证明这些值不含 caller heap handle且不会造成 caller context借用跨 wait。

### 2. 普通 DB operation

`eval_program_db_operation_with_context` 必须按下列阶段执行：

```text
递归 prepare DbCommand
  -> 构造 caller-heap-free operation
  -> actual-Pending wait
  -> resume 后 finalize到 caller heap
```

具体要求：

- `DbIrEvaluator::eval_operation` 的现有求值、验证和错误顺序不变；
- raw/wire command在 wait前拥有 type、selector/query/order/projection、document/change等输入；
- raw DB wait返回 owned `DbDocument`、page、count/bool或 provider error，decode只在 wait后发生；
- recoverable find/create/update/replace必须消费 O5R2六个 prepared入口及同一个 finalizer；
- `limit: 0`短路、required-not-found、null、result plan、boundary use和recoverable context不变；
- Ready与Pending都只启动一次；provider error或finalizer error不得重放数据库副作用；
- finalizer失败沿 O5R2 checkpoint规则收束，不让 caller heap保留半物化结果；
- 不为普通 operation新建 detached Actor frame、heap clone、heap mutex或全局 operation registry。

旧 `execute_db_command` 可以拆成窄 command prepare/wait/finalize模块；不得保留一套新状态机和一套
旧 heap-borrowing runtime状态机并行。

### 3. `DbQuery` 明确不释放

`eval_program_db_query_value` 继续只调用 `DbIrEvaluator::eval_query_value`：

- 不把整个 query evaluator包装进 `await_if_pending`；
- 不在进入 query前预释放 Actor segment；
- query内若有 sleep、service、native或其它真正外部 wait，由对应嵌套 expression自己的 owner
  观察 actual-Pending；
- 加真实 Actor fixture证明纯 query Ready路径的 segment计数不变。

### 4. Transaction 多阶段状态机

legacy call形式与显式 `DbTransactionIr` 必须共用同一 transaction lifecycle owner：

```text
begin external wait
  -> body/result evaluator
  -> commit external wait
  -> success

begin成功后的任意 body/result/flow/commit error、cancel或future drop
  -> exactly-once abort
  -> 现有 checkpoint truncate
  -> 原 error/terminal
```

要求：

- begin、commit、abort分别是独立 caller-heap-free external wait，并各自通过 actual-Pending；
- body保持正常递归 evaluator；其中每个嵌套 operation自行处理自己的 Ready/Pending；
- commit失败后必须 abort一次，不能再次commit；
- body error、非法 flow、result expression error均 abort一次；
- begin失败时不得abort一个未开始的 transaction；
- normal commit后不得abort；
- outer evaluator future在begin成功后被drop/cancel时，transaction owner必须保证 abort最终只执行
  一次，且不能无限等待或依赖重启整个 evaluator；
- lifecycle owner必须有显式阶段/terminal所有权，不能用多个松散 bool制造 double terminal；
- checkpoint只保持现有 allocation truncate语义，不声称撤销 transaction body对既有 heap节点
  的原地 mutation。

不得把 begin → body → commit/abort 整体包装成一个借 `&mut RequestHeap`/`Env` 的 future后交给
E3。若当前 store接口无法在 drop/cancel下安全保证 exactly-once abort，必须
`TASK_SCOPE_EXPANDED`，不得用无界 detached task、阻塞 Drop或忽略 abort规避。

### 5. Lease claim 多阶段状态机

`eval_program_db_lease_claim` 必须显式拥有：

```text
key prepare
  -> claim external wait
  -> binding import + renew owner
  -> body evaluator
  -> stop/join renew owner
  -> lease_lost external wait
  -> release external wait
  -> exactly-once terminal
```

要求：

- key wire encode在 claim wait前完成；
- claim返回 `None`时返回 `false`，不启动 renew、不release；
- claim成功后才把 binding decode/import到 caller heap/env；
- renew task只拥有 clone store、hold、period和 cancellation carrier，不借 caller heap/env；
- body内部每个外部 operation按各自 actual-Pending运行；
- normal、body error、非法 flow、cancel和drop均停止 renew并release一次；
- renew失败继续沿当前语义标记 request cancellation；lease-lost优先于正常 body success；
- release error与body error的优先级保持现有可观察行为；不得吞掉 lease-lost；
- late renew结果不得在 terminal后重新改变状态或启动第二次release；
- 不允许只 `abort()` renew handle而从不证明其 terminal/资源收束。

若 renew owner不能可靠停止，或 future drop/cancel无法保证 lease exactly-once release，必须
`TASK_SCOPE_EXPANDED`；不得复制 Actor scheduler或把 caller heap移进 renew task。

### 6. Lease read

`eval_program_db_lease_read` 按简单 prepare/wait/finalize执行：

- 递归求值 key并在 caller segment编码为 owned `DbKey`；
- `read_lease` 是 caller-heap-free actual-Pending wait；
- resume后才把 wire value decode到 caller heap；
- `None`仍返回 `Null`；
- Ready/Pending各只启动一次，drop/error不重试。

### 7. Checkpoint 与错误

- 每个外部 wait进入与恢复后使用当前已有 owner-aware checkpoint位置；本节点不得设计新的
  timeout/cancellation协议，也不得修改 E1 scope owner。
- DB provider、decode、lease-lost、illegal-flow和普通 evaluator error的现有
  `RuntimeError`形状保持不变。
- cancellation继续是内部 terminal，不得重新包装成可 catch payload。
- first poll返回 error属于Ready：不得释放 Actor segment。
- first Pending后返回 error必须先由E3恢复/收束Actor segment，再把原error交给DB状态机。

## Test-first 与验收

先增加真实 RED，再实现。测试不得只测一个抽象 helper；至少穿过真实
`eval_program_db_*`入口和真实 Actor frame/store fixture。

至少覆盖：

1. 纯 `DbQuery` Ready：不 commit/release/reacquire；
2. raw ordinary operation Ready与Pending：副作用各一次，Pending只切一个segment；
3. recoverable ordinary operation Ready与Pending：消费 O5R2 prepared wait/finalizer；
4. Pending期间 caller heap可独立使用，finalize前不变，resume后结果可见；
5. provider error first-Ready与Pending-after-error都不重启；
6. transaction begin/body operation/commit分别可真实Pending，segment数与阶段一致；
7. transaction begin failure不abort，normal commit不abort，其余 error/非法flow/commit失败/cancel/
   drop均abort exactly once；
8. 两种 transaction source形状共用 lifecycle owner；
9. lease claim `None`、Ready与Pending；
10. claim成功后的binding、renew、body Pending、lease-lost、release error；
11. normal/error/非法flow/cancel/drop停止 renew并release exactly once；
12. lease read Ready/Pending/None/decode error；
13. 同一个 wait future不被重建；start、poll、terminal、drop计数精确；
14. Actor field在 first poll前发生的合法 heap mutation于Pending提交后仍可见，且Ready路径不切换；
15. replacement/identity fence失败沿既有E3 fail-closed，不安装 stale lease。

优先将 fake store、gate future、计数器和 Actor fixture放入窄 child test模块，不能继续把
`program_db.rs`或`db_eval.rs`堆成更长的混合文件。

使用 worktree专属 target：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6-eval-db/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6-eval-db/build/cargo-target \
  cargo test -p skiff-runtime-eval db_actor -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6-eval-db/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6-eval-db/build/cargo-target \
  cargo check -p skiff-runtime-eval -p skiff-runtime-service-db \
    -p skiff-runtime-capability-context --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6-eval-db/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录每个 selector实际测试数；零测试不算证据。不得连接真实 MongoDB、运行 stable/live或访问
网络。

反向检查必须证明：

- 普通 recoverable DB不再调用六条 heap-borrowing `*_runtime(..., heap).await`；
- transaction/claim不是一个借 caller heap/env的整段 wait；
- 没有 pre-suspend、`yield_now`、unsafe heap alias、heap mutex、future restart或 copied Actor
  scheduler；
- `DbQuery`没有新增外部 suspension wrapper。

## 写集与结构

只允许：

- `runtime/eval/src/program_db.rs`
- `runtime/eval/src/program_db/**`
- `runtime/eval/src/db_eval.rs`
- `runtime/eval/src/db_eval/**`
- 本 result

`program_db.rs`和`db_eval.rs`都已经很长。root只保留现有入口、module声明和薄转发；新增 ordinary
operation、transaction、lease lifecycle、fixture与测试矩阵必须按职责进入child module。新文件
若增长到数百行且同时承担多个owner，应继续拆分。

不得修改：

- `runtime/eval/src/eval_context.rs`及其它E4R call-site owner；
- E1/E2/E3 Actor/scheduler owner；
- service-db、capability-context、native、service/Actor/callback owner；
- compiler、artifact、linker、Router、Cargo manifest或lockfile；
-语言、DB、transaction、lease、timeout、error或wire语义。

若实现需要上述写集外 production改动、公共 API变化、复制E3状态机，或 transaction/lease的
cancel/drop语义无法在当前接口下满足，立即停止并精确提交 `TASK_SCOPE_EXPANDED`。发现一个直接
小缺陷时记录最小RED、准确owner与建议修正，不得越界绕过。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6-eval-db
branch   codex/p5-f445h-o6-eval-db
```

先提交 implementation，再单独提交
`P5-F445H-O6-evaluator-db-state-machines-result.md`。最终worktree clean；不得
merge/rebase/push。

本任务范围已明确，不派子 Agent。若探查后实际范围超出合同，或仍有多个不明确问题，结束任务并
如实上报，不自行扩大范围。
