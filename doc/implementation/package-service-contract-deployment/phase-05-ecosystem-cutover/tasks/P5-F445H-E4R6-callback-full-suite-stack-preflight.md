# P5-F445H-E4R6 callback full-suite stack preflight

状态：Ready。E4R5 独立验收发现的唯一 blocker只读探查。目标是定位
`f445h_e4r_spine_callback_pending_reacquires_before_finalize` 仅在完整 lib suite中栈溢出的
原因，并冻结一个可执行修复节点；本任务不修代码。

## 直接父节点与冻结事实

- `P5-F445H-E4R5-combined-integration-acceptance-result.md`
- `P5-F445H-E4R1-evaluator-spine-actual-pending-checkpoint-result.md`
- `P5-F445H-J1-prepared-operation-combined-review-result.md`

冻结 production/tests候选为
`da49c17cb6e3c479ea649b936aab8614d3beface`。其 combined 5/5、locked check、fmt、diff和
12项静态检查均通过；唯一完整 gate blocker为：

```text
actor_executor::tests::actor_concurrent_continuation::
evaluator_actual_pending::callback_matrix::
f445h_e4r_spine_callback_pending_reacquires_before_finalize
```

完整 lib binary inventory为395，该 test发生 stack overflow并 `SIGABRT`。相同 selector在 R1
focused matrix中曾通过，因此不能直接假设是真实 production无限递归，也不能把增加线程栈当作
默认修复。

## 角色与唯一写集

唯一允许写入：

- 新增 `P5-F445H-E4R6-callback-full-suite-stack-preflight-result.md`

production、tests、fixture、Cargo/manifest/lockfile及其它文档只读。可以把命令日志写到
worktree `build/` 下作为临时证据，不提交。不得修改或临时patch源码，不得派子 Agent。

## 必须回答

1. 精确 test单独运行是否稳定通过；默认test线程、`--test-threads=1`是否有差异；
2. 完整 lib suite在串行与默认/有界并行下是否分别通过或栈溢出；
3. failure是否依赖：
   - test并发；
   - 某个先行/同时运行的测试或全局可变fixture；
   - test线程栈阈值/巨大future；
   - callback owner真实递归；
   - 其它精确条件；
4. 栈溢出发生在测试 harness、callback test-support、prepared callback owner还是 production
   evaluator route；
5. 最小修复写集、唯一owner和可执行 RED/GREEN验收是什么；
6. 修复是否只需 test结构/隔离，还是会改变 production语义或公共 owner。

结论必须有重复可验证的命令/路径证据，不能只根据“focused通过、full失败”猜测。

## 有界诊断顺序

使用独立 target：

```text
/Users/geek/workspace/skiff-p5-f445h-e4r6-preflight/build/cargo-target
```

先做只读代码和静态状态检查，再按最小成本执行：

1. 精确 failing test单独运行，至少确认一次默认和一次
   `--test-threads=1`；
2. 完整 `--lib` suite串行运行一次；
3. 只有为区分并发/顺序必要时，再运行一次默认或小并发完整 lib suite；
4. 若证据指向线程栈大小，可用一次明确 `RUST_MIN_STACK` 对照；不得把环境变量本身当修复；
5. 若证据指向测试碰撞，使用 test名称集合、`--skip`、小组/二分或明确同步点找出最小碰撞面；
6. 检查 callback test及其调用链中的 static/global registry、test double、Tokio runtime、
   callback capability registration、request generation和递归入口。

完整 lib execution最多三次；已有 E4R5 full failure不需要无意义重复。不要运行 combined/full
integration gate、其它仓库、stable、live、network或 MongoDB。

若 stack overflow无法提供 backtrace，可使用进程退出、test顺序、线程数、栈阈值和最小碰撞
集合形成因果证据；不得因为没有backtrace就虚构具体frame。

## Result要求

状态只能是：

- `READY_FOR_E4R6_FIX`：已形成单一原因、唯一owner、精确写集和 RED/GREEN；
- `TASK_SCOPE_EXPANDED`：根因要求修改公共 owner/语义或多个独立DAG节点；
- `TASK_NOT_EXECUTABLE`：完成有界探查仍无法形成安全修复路径。

result至少记录：

- 当前 commit/tree与clean状态；
- 每个诊断命令、test数量、exit和是否 stack overflow；
- 单独/串行/并行/栈大小或碰撞矩阵；
- 精确代码路径和调用链；
- 被排除的假设及证据；
- 唯一修复owner与允许写集；
- 修复任务的真实 RED、focused GREEN和一次完整 lib重验要求；
- 是否需要用户决策；
- 未决问题与残余风险。

如果发现一个显而易见的小修，也只能在result中给出，不得本任务直接实现。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r6-preflight
branch   codex/p5-f445h-e4r6-preflight
```

只提交result；返回result commit、状态、根因证据、修复写集和clean worktree。不得
merge/rebase/push。

风险：高。此探查结果决定完整 gate blocker的唯一修复owner。
