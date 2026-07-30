# P5-F445H-J1 prepared operation combined review

状态：Ready。O1–O6 的 operation owner 已全部合流，O6 DB/Actor combined acceptance 已关闭。
本节点按 E3R 冻结的 DAG 做一次独立只读 join review，判断 E4R 是否可以只迁移 evaluator call site，
而无需继续修改任何 operation owner。

## 直接父节点

- `P5-F445H-O1-native-prepared-external-operation-result.md`
- `P5-F445H-O2-outbound-actor-prepared-operation-result.md`
- `P5-F445H-O3-in-process-service-prepared-operation-result.md`
- `P5-F445H-O4-callback-prepared-state-machine-result.md`
- `P5-F445H-O5R2-service-db-prepared-runtime-operation-result.md`
- `P5-F445H-O6R13-db-actor-combined-acceptance-result.md`
- `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md`

引用链继续追溯到 E1/E2/E3/E23 与唯一权威设计。冻结代码候选为 integration commit
`4a6c70b9`；本任务合同提交只新增文档，不改变代码候选。

## 角色与写集

这是 E4R prerequisite 的独立只读验收，不是 implementation，也不是整个 F445H gate。唯一允许
写入：

- `P5-F445H-J1-prepared-operation-combined-review-result.md`

不得修改 production、tests、fixture、既有 task/result、Cargo、manifest 或 lockfile；发现问题时
返回 `FAIL` 和精确 owner，不得修复。不得 merge/rebase/push，不得派子 Agent。

风险：高（多个 operation owner 在 actual-Pending seam 前的所有权 join）。

## 必须独立审查

### 1. Native owner

- `PreparedNativeCall` 明确区分 `Ready` 与 `ExternalWait`；
- external wait 不借 caller `RequestHeap`、`Env` 或 `EvalContext`，finalize 才重新接收 heap；
- sleep zero由真实 first poll决定，不能按 binding 静态判定；
-四个 WebSocket send 为同步 `Ready`；
- HTTP/file/requestJson/Actor registry 的 side effect只启动一次，drop guard不重启；
- native owner 不读取 `may_suspend`、不含 `native_call_suspends` 或 pre-suspend。

### 2. Outbound service、remote interface 与 Actor invocation

- dependency 与 remote interface 共用同一个 outbound prepared request owner；
- unary wait拥有 lease/receiver，不借 caller heap/env；finalize后才 decode/coerce；
- `serverStream` setup 返回同步 Ready，lease/receiver转交 source，等待只发生在 consumer next；
- Actor prepare完成 argument encode，wait只持 owned request/context，finalize后才 import；
- drop/cancel/late response只结算一个 owner，副作用不重放。

### 3. Activation-relative provider

- provider unary 分成 caller-side prepare、owned provider wait、caller finalize；
- wait只拥有 provider heap/context/env/request，不捕获 caller heap/env/Actor frame；
- Ready/Pending使用同一 wait，不按 service kind预释放；
- provider serverStream setup/producer/consumer lifecycle没有被 unary protocol吞并。

### 4. Callback

- callback owner heap通过 owned mutex guard持有，不借 caller heap/env/context；
- prepared wait不捕获 caller Actor frame，递归 owner evaluator只执行一次；
- parameter prepare失败恢复本次 checkpoint；method error/cancel/drop释放 guard且不伪造 rollback；
- finalize只消费一次 completed outcome，不重复 import。

### 5. Service DB 与 eval DB

- 六个 service-db prepared runtime入口的 owned wait不保存 caller heap/heap handle；
- finalizer才重新接收 caller heap，失败回滚本次物化；
- eval raw/prepared、transaction和lease按 O6R13 的单一状态机使用相同 owned wait；
- `DbQuery`没有外部 wait，不需要 Actor cut；
- pending drop不重建 DB command/future；transaction/lease不 detached cleanup。

### 6. E4R 可执行性

逐一核对 evaluator 当前仍 pre-suspend 的 call site，建立：

```text
call site
  -> 已存在的 prepare API
  -> Ready 或 heap/env-free wait
  -> E3 await_if_pending
  -> 已存在的 finalize API
```

必须覆盖：

- native；
- legacy service dependency；
- remote interface；
- Actor method；
- activation-relative unary service；
- callback；
- DB operation/transaction/lease/read；
- stream emit/next。

同步例外必须明确：

- WebSocket send不释放；
- serverStream创建不释放，消费 next按真实 Pending；
- DbQuery不释放；
-其它 prepared `Ready`不释放。

当前 `eval_context.rs` 中 `native_call_suspends` 和 pre-suspend pair 尚存在是 E4R 的预期 RED，不是
J1 blocker。只有 operation owner仍要求这些 pre-suspend、wait仍借 caller state、或 finalize不能在
resume后执行，才判 FAIL。

## 聚焦证据与唯一 owner

先用 selector listing/代码事实确认命令非零，再运行以下互不替代的 focused suites。不要重跑 O6R13
已在同一代码候选上拥有的 `program_db`、`db_actor_` 或完整 eval gate。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-j1/build/cargo-target \
  cargo test -p skiff-runtime-native dispatch -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-j1/build/cargo-target \
  cargo test -p skiff-runtime-eval service_dispatch -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-j1/build/cargo-target \
  cargo test -p skiff-runtime-eval actor_dispatch -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-j1/build/cargo-target \
  cargo test -p skiff-runtime-eval async_stream_cancel -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-j1/build/cargo-target \
  cargo test -p skiff-runtime-eval callback_native -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-j1/build/cargo-target \
  cargo test -p skiff-runtime-native callback_adapter -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-j1/build/cargo-target \
  cargo test -p skiff-runtime-service-db prepared_runtime -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-j1/build/cargo-target \
  cargo test -p skiff-runtime-capability-context prepared_db -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-j1/build/cargo-target \
  cargo check -p skiff-runtime-native -p skiff-runtime-eval \
    -p skiff-runtime-service-db -p skiff-runtime-capability-context --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-j1/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录每个主 selector 的实际执行数；后续 integration binary 零匹配不算失败，但主 selector必须非零。
不得运行 full eval/native/service-db suite、MongoDB、stable、live 或 network。O5R2 已记录的真实
Mongo与旧 namespace fixture限制不是本节点要绕过或修复的内容。

## Verdict

结果必须包含：

- `PASS / E4R_EXECUTABLE` 或 `FAIL / <owner blocker>`；
- blocking issues、non-blocking follow-up；
- 每个 owner 的精确类型/方法路径及 borrow/lifecycle结论；
- evaluator call-site→prepared owner映射；
- 命令、实际计数、代码候选；
- residual risk。

若 PASS，明确：

- J1只证明 owner prerequisite闭合；
- E4R仍需删除 pre-suspend、接线 timeout/concurrent/catch/checkpoint/stream并运行自己的完整 gate；
- I6仍未开始。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-j1
branch   codex/p5-f445h-j1
```

只提交 result 文档，worktree clean。
