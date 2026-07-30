# P5-F445H-O6R12 lease renew stop race

状态：Ready。O6R11 证明：正常 claim 收尾发送 stop 后，如果 renew future 已进入 actual-Pending，
当前 `LeaseRenewOwner` 会永远卡在 renew await，无法 join，也无法继续 LeaseLost→Release。本节点只修复
这个 production renew/stop 竞争。

## 直接父节点

- `P5-F445H-O6R11-db-actor-lease-matrix-final-result.md`
- `P5-F445H-O6R-evaluator-db-internal-stop-state-machines-result.md`
- `P5-F445H-D1-internal-execution-stop-semantics-result.md`

引用链追溯到唯一权威设计。当前 integration checkpoint 为 `afc8d441`；其中 tests commit
`814f3923` 保留 14 个 lease 矩阵函数和精确 RED：

```text
db_actor_lease_body_pending_cleanup_stops_renew_before_terminals
```

该 RED 当前以一秒 timeout 确定失败，phase 停在
`Claim -> BodyCreate -> Renew`，Renew 为 actual-Pending 且没有 drop，LeaseLost/Release 为零。
不得删除、忽略、放宽或改写该 RED。

## 已冻结语义与 owner

权威语义：

- 正常成功、业务错误和非法 flow 都必须停止并 join renew owner，之后读取 LeaseLost 并等待 Release；
- 异常 outer drop 仍由 `LeaseRenewOwner::Drop` 同步 abort task，不 await、不 spawn cleanup；
- renew 失败在其 terminal 先被观察到时继续设置 request 内部 stop/lease-lost 状态；
- stop 是运行时内部生命周期信号，不是公开请求取消，不产生 `CancelError` 或业务错误；
- stop 与同一时刻的 renew terminal 同时 ready 时，stop 优先，晚到 renew 结果被丢弃；
- normal `stop_and_join` 必须等 pending renew future 已 drop、renew task 已退出后才返回。

唯一 production owner 是 `runtime/eval/src/program_db/lease.rs` 中
`LeaseRenewOwner::start` 的 task loop。当前外层 select 只竞争 stop 与 interval tick；tick branch 内部
直接 await renew，造成 stop 无法参与正在进行的 renew。

## 唯一写集

- `runtime/eval/src/program_db/lease.rs`
- `P5-F445H-O6R12-lease-renew-stop-race-result.md`

不得修改 lease/transaction/ordinary matrix、fixture、`program_db.rs`、wait/transaction owner、
Actor E3、capability-context、service-db、driver tests、Cargo、manifest 或 lockfile。

## 必须实现

保持一个 renew task、一个 stop carrier 与现有 RAII owner。最小修正 task loop：

1. interval 等待期间，stop/closed watch 仍能立即结束；
2. interval 触发并构造一个 renew future 后，必须继续在同一个 task 内竞争：
   - stop/closed watch：优先选择，drop 当前 pending renew future并退出；
   - renew terminal：调用既有 `handle_renew_result`；成功继续下一 tick，false/error 设置内部 stop后
     退出；
3. 不构造第二个 renew future，不 detach、不 spawn cleanup；
4. `stop_and_join` 仍通过 stop signal 后 await同一个 `JoinHandle`，不得改成 normal-path
   `JoinHandle::abort()`；
5. outer `Drop` 仍同步 `abort()` 尚存 task；
6. 不改变 tick period、公开 API、error type或 lease terminal 优先级。

可以在同一文件的现有 unit-test module 增加一个窄 test，但不得新建 fake store surface；O6R11 的真实
claim/store/Actor RED 是权威动态证据。

若修复需要改 watch carrier、claim evaluator、DB capability API或 service-db，立即返回
`TASK_SCOPE_EXPANDED`，不得扩大写集。

## RED→GREEN 与验证

先在未改 production 的起点运行精确 RED，确认 1 个测试按 timeout 失败；修改后原样转 GREEN，并且
Renew metrics 必须为：

- constructed 一次；
- 至少一次 Pending；
- normal stop 后 `dropped_before_terminal == 1`；
- `ready_returns == 0`；
- 随后 LeaseLost 与 Release 各构造/完成一次。

验证：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r12-renew-stop/build/cargo-target \
  cargo test -p skiff-runtime-eval \
    program_db::tests::lease::db_actor_lease_body_pending_cleanup_stops_renew_before_terminals \
    -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r12-renew-stop/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::lease:: -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r12-renew-stop/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::lease::tests:: -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r12-renew-stop/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r12-renew-stop/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试数；lease matrix 必须 14/14，production lease unit selector 必须非零。反向检查只保留
一个 `tokio::spawn` renew owner、normal path 不调用 abort、outer Drop 仍调用 abort。

不运行完整 eval/stage gate、stable、live、network 或 MongoDB。不得 merge、rebase 或 push。

## 执行与证据

风险：高（正常生命周期 join 与 pending operation cancellation）。这是新的 production checkpoint，
完成后只解除 O6R11 重验，不代表 combined acceptance。启动后五分钟内首次修改 `lease.rs`；此前只
确认精确 RED，不重做设计或开放式扫描。不得派子 Agent。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6r12-renew-stop
branch   codex/p5-f445h-o6r12-renew-stop
```

先提交 production+GREEN，再单独提交 result；返回两个 commit、自验收矩阵与未决问题。worktree
clean。证据锚定 `afc8d441`；lease production、fixture、Actor E3 或相关依赖变化会使证据失效。
