# P5-F445H-O6R12 lease renew stop race result

状态：`PASS / PRODUCTION_RENEW_STOP_RACE_FIXED / O6R11_REVALIDATION_UNBLOCKED`。

证据锚定 integration checkpoint `afc8d441` 与 O6R11 tests commit `814f3923`。production
提交为 `4e45366a`。本节点只修复 `LeaseRenewOwner::start` 中 pending renew 与 normal stop 的竞争；
它解除 O6R11 重验 blocker，不声明 combined acceptance 已完成。

## 1. Production 修复

`runtime/eval/src/program_db/lease.rs` 保留原有 RAII owner、单一 watch stop carrier 和单一 renew
task，并把每次 tick 后的 renew 生命周期收进同一个 task：

1. interval 等待仍由外层 biased select 与 stop/closed watch 竞争；
2. tick 只构造一个 `renew_lease` future并 pin；
3. 内层 biased select 把 stop/closed watch 放在 renew terminal 前；命中 stop 时退出并 drop 当前
   pending future；
4. 非 stop 的 watch change 继续 poll 同一个 pinned future，不重建；
5. renew terminal 仍调用既有 `handle_renew_result`；成功进入下一 tick，false/error 设置 request
   内部 stop 状态后退出。

`stop_and_join` 仍只发送 stop 并 await 同一个 `JoinHandle`，没有 normal-path abort。outer
`LeaseRenewOwner::Drop` 仍同步 abort 尚存 task。没有新增 task、detach、cleanup spawn、公开 API、
error type或 period 变更。

## 2. RED→GREEN

修改 production 前原样运行精确测试：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r12-renew-stop/build/cargo-target \
  cargo test -p skiff-runtime-eval \
    program_db::tests::lease::db_actor_lease_body_pending_cleanup_stops_renew_before_terminals \
    -- --nocapture
```

RED 按预期在一秒 timeout 后失败：

```text
phases=[Claim, BodyCreate, Renew]
Renew: constructed=1 polls=1 pending_returns=1 ready_returns=0
       dropped_before_terminal=0 dropped_after_terminal=0
LeaseLost: constructed=0
Release:   constructed=0
test result: FAILED. 0 passed; 1 failed; 0 ignored
```

production 修复后同一命令原样转 GREEN：

```text
running 1 test
phase=body variant=normal-success
phase=body variant=body-error
test result: ok. 1 passed; 0 failed; 0 ignored
```

O6R11 的测试与 fixture 未修改。通过的既有断言对两个 variant 均验证：

- Renew `constructed == 1`；
- Renew `pending_returns > 0`；
- Renew `ready_returns == 0`；
- Renew `dropped_before_terminal == 1`、`dropped_after_terminal == 0`；
- LeaseLost 与 Release 各构造并完成一次；
- terminal phase 顺序为 `Renew -> LeaseLost -> Release`；
- 放行晚到 renew sender 后 metrics 不再变化。

## 3. 验证结果

| 验证 | 实际结果 |
| --- | --- |
| 精确 O6R11 RED→GREEN | `1/1` PASS；两个 body variant 均完成 |
| `cargo test -p skiff-runtime-eval program_db::tests::lease:: -- --nocapture` | `14/14` PASS，`314` filtered out |
| `cargo test -p skiff-runtime-eval program_db::lease::tests:: -- --nocapture` | `3/3` PASS，非零 selector |
| `cargo check -p skiff-runtime-eval --tests --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

Cargo 输出包含仓库既有 unused/dead-code/unreachable-pattern warnings；本节点没有修改对应文件，也没有
新增 warning。依任务边界，未运行完整 eval/stage gate、stable、live、network 或 MongoDB。

## 4. 自验收矩阵

| 任务条款 | 代码/动态证据 | 结果 |
| --- | --- | --- |
| interval 等待可被 stop/closed 立即结束 | 外层 biased select 保留 watch-first 分支 | PASS |
| pending renew 期间 stop 优先 | 内层 biased select 的 watch 分支位于 renew terminal 前 | PASS |
| stop drop 当前 pending renew 并等待 task 退出 | 精确 GREEN 验证 Renew 单次 Pending drop，随后 Lost/Release；`stop_and_join` await task | PASS |
| renew terminal 保留既有处理 | terminal 分支调用 `handle_renew_result`；production unit `3/3` | PASS |
| 一个 renew future、一个 renew task | 每个 tick 构造并 pin 一次；production 仅一个 `tokio::spawn` | PASS |
| normal path 不 abort | `stop_and_join` 只有 `stop.send(true)` 与 `task.await` | PASS |
| outer drop 同步 abort | `Drop` 中仍有唯一 production `task.abort()` | PASS |
| 不改变公开语义 | owner API、period、error type、terminal handling均未改 | PASS |
| lease matrix 不回归 | O6R11 selector `14/14` | PASS |
| 唯一写集 | production commit 只改 `lease.rs`；本文单独提交 | PASS |
| 禁止边界 | 未改 matrix/fixture、`program_db.rs`、其它 owner、Actor E3、capability/service-db、Cargo/manifest/lockfile | PASS |
| Git/环境边界 | 未 merge/rebase/push；未启动外部服务 | PASS |

反向检查 production 区域只找到一个 `tokio::spawn`、一个 `renew_lease` 构造点和仅位于 outer
`Drop` 的 `task.abort()`；normal `stop_and_join` 未出现 abort。

## 5. 未决问题

本节点没有 scope 内未决问题。O6R11 可在集成后基于 production commit `4e45366a` 重验；若 lease
production、fixture、Actor E3 或相关依赖在集成前发生变化，本证据失效，必须重新运行同一组聚焦验证。
本结果不替代 phase combined acceptance。
