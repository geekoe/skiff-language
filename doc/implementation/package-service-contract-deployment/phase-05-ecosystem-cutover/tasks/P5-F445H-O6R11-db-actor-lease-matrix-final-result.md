# P5-F445H-O6R11 DB/Actor lease matrix final result

状态：`TASK_SCOPE_EXPANDED / PRODUCTION_RENEW_STOP_BLOCKER / MATRIX_NOT_GREEN`。

production prerequisite 为 `2d5df5ae`，重新冻结的 fixture checkpoint 为 `637567f3`，tests 证据提交为
`814f3923`。本节点实际保留 14 个非零 `db_actor_lease_*` 测试函数，并完成 claim/read/drop/terminal
矩阵的 test-only 实现；但真实 Renew future 在 normal cleanup 的 `stop_and_join` 路径无法被停止，
修复需要修改唯一写集之外的 `runtime/eval/src/program_db/lease.rs`。依叶子任务停止条件，本节点返回
`TASK_SCOPE_EXPANDED`，不修改 production、fixture 或其它 owner。

## 1. 硬停止原因

最小失败 case 使用 frozen claim、`BODY_CREATE_BLOCK_LABEL`、真实 Actor frame、production
`eval_program_db_lease_claim` 与 fixture store：

1. Claim first-Ready 成功并导入 frozen binding slot；
2. body create 返回 actual-Pending，同时真实 Renew future 完成 first poll 并保持 Pending；
3. body gate 放行后，同一 body future 返回 Ready；
4. production 进入 `renew_owner.stop_and_join()`，但 Renew future 没有 drop，后续 LeaseLost/Release
   永远不构造。

一秒有界探针记录：

```text
phases=[Claim, BodyCreate, Renew]
BodyCreate: constructed=1 polls=3 pending=2 ready=1 dropped_before_terminal=0
Renew:      constructed=1 polls=1 pending=1 ready=0 dropped_before_terminal=0
LeaseLost:  constructed=0
Release:    constructed=0
```

production `LeaseRenewOwner` 的 stop select 只包围 interval tick；tick 分支选中后，会在分支内部直接
await `store.renew_lease(...)`。因此 Renew 已 actual-Pending 时，stop watch 不再参与竞争，
`stop_and_join()` 只能永久等待 task。修复必须让 pending renew 与 stop 信号可取消竞争，这属于
`runtime/eval/src/program_db/lease.rs` 的 production 改动，超出本节点唯一写集。

## 2. 最小失败证据

tests commit 中
`db_actor_lease_body_pending_cleanup_stops_renew_before_terminals` 对 normal-success/body-error 共用真实
生命周期探针，并用一秒 timeout 把无限等待收敛为确定失败。精确执行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r11-lease/build/cargo-target \
  cargo test -p skiff-runtime-eval \
  program_db::tests::lease::db_actor_lease_body_pending_cleanup_stops_renew_before_terminals \
  -- --nocapture
```

实际结果：

```text
running 1 test
phase=body variant=normal-success
claim cleanup did not terminate for normal-success
test result: FAILED. 0 passed; 1 failed; 0 ignored
```

此前全 selector 的最终有界编排已观察其它 13 个函数返回 `ok`，只剩该 normal cleanup 挂起；发现该
production blocker 后中止无界进程并改为上述最小 timeout probe，没有继续运行完整 selector 或其它
昂贵 gate。

## 3. 已实现矩阵与状态

| 必须覆盖 | test-only 证据 | 状态 |
| --- | --- | --- |
| Claim None Ready / actual-Pending | binding 不可见；Renew/Lost/Release 均未启动；Pending 竞争 Actor segment | PASS |
| Claim success Ready / actual-Pending | `Env::for_program_executable`；成功前 slot 未初始化，成功后 Heap binding 可见；Claim 单次 | PASS |
| normal success / body error cleanup | frozen body-create、真实 Renew first-poll 与 stop/join ordered trace | BLOCKED：pending Renew 不可停止 |
| explicit illegal flow cleanup | frozen illegal-flow block；Lost→Release 后返回既有 flow error | PASS |
| LeaseLost Ready / actual-Pending 与 Release error priority | Lost 胜 Release；Release 胜 body flow；每个 operation 单次 | PASS |
| body DB Pending outer-drop | 真实 Renew 被 abort/drop；late gates 不增长 poll、不物化 body；Release 为零 | PASS |
| Release Pending outer-drop | pending Release 单次 drop；late sender 无 Ready、无重建、无 heap 变化 | PASS |
| Read Ready / Pending / None / store error | production read evaluator、真实 Actor segment、单一 future 与 phase metrics | PASS |
| Read object/array decode error | `RequestHeapLimits { max_nodes: 0, .. }`；无 heap node 物化 | PASS |
| Read Pending outer-drop | Read 单次 drop；late sender无 Ready、重建、物化或其它 phase | PASS |
| 至少 8 个非零函数 | `rg` 实际计数 14 | PASS |
| 完整 selector GREEN | normal cleanup production blocker | FAIL |

## 4. 自验收与边界

| 任务条款 | 证据 | 结果 |
| --- | --- | --- |
| 保留两个既有 GREEN | fixture binding probe 与 claim-None pending regression 未删除、忽略或弱化 | PASS |
| 唯一 fixture / production evaluator | 只消费 frozen linked IR、labels、phase scripts、metrics/probe 与 Actor frame | PASS |
| 不直测 Renew owner / 不复制 fake | lease matrix 仅经 production claim/read evaluator 触发真实 store methods | PASS |
| ordered trace / metrics / binding / Actor segment | 每个组合 variant 打印 phase/variant 并独立断言 | PASS，除 blocker 后不可达 terminal |
| 实际方法证据 | Claim、Read、Renew、LeaseLost、Release 均有非零 constructed/poll case | PASS |
| stop condition | production 缺陷收敛后停止，没有越权修复或降低断言 | PASS |
| 唯一写集 | tests commit 仅 `runtime/eval/src/program_db/tests/lease.rs`；本文单独提交 | PASS |
| 禁止边界 | 未修改 fixture、ordinary/transaction、production、Actor E3、capability-context、service-db、Cargo/manifest/lockfile | PASS |
| 环境边界 | 未运行 stable、live、network、MongoDB 或完整 eval/stage gate；未 merge/rebase/push | PASS |

`git diff --check` 在 tests 提交前通过。任务指定的 `cargo check --tests --locked` 与
`cargo fmt --check` 未继续执行：production blocker 已命中“五分钟内停止”条件；tests 文件已由
`rustfmt --edition 2021` 格式化。

## 5. 解除条件与未决问题

production owner 需要在独立写集中修复 `LeaseRenewOwner`：stop 信号必须能取消已经进入
actual-Pending 的 `renew_lease` future，并保证 `stop_and_join` 等待该 future drop 后再进入
LeaseLost→Release。修复集成后，本节点需要基于新的 production commit 重跑：

```text
cargo test -p skiff-runtime-eval program_db::tests::lease:: -- --nocapture
cargo check -p skiff-runtime-eval --tests --locked
cargo fmt --check
git diff --check
```

fixture、lease production、Actor E3 或相关依赖变化都会使本证据失效。本节点不声明 combined
acceptance 已解除。
