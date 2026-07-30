# P5-F445H-E1 Eval scope, terminal and checkpoint core

状态：Ready。F445H DAG 的 eval current-scope core。

## 直接父节点

- `P5-F445H-eval-concurrency-owner-preflight-result.md`
- `P5-F445H-R0-lease-child-execution-scope-result.md`

## 完成目标

### 1. Current owned execution control

`ProgramExecutionContext` 内部持有 `OwnedExecutionControl`，构造时从 input borrowed control 立即
capture。必须提供 crate-private：

- 每次 invocation 读取 current borrowed `execution()`；
- 在 context clone 上安装 child owned control；
- 从 current control读取完整 `ExecutionScope`；
- derive timeout child context，不原地替换 parent。

normal、error、timeout、cancel或 future drop 都通过 child context值所有权自然恢复 parent。
`OwnedProgramExecutionContext::{capture,borrow}` 必须保留调用时的 current control，不退回
request-start snapshot。

本节点不重建 time/file/websocket/HTTP adapter；I6/E4 后继在每次 invocation 从 current
`execution()` / `execution_scope()` 读取。

### 2. Monotonic clock 与 duration

在 `program_execution` child module 提供 eval-private monotonic clock seam：

- production 使用 monotonic `Instant`；
- tests 可使用 scripted clock，在第 N 次 checkpoint越过 deadline；
- `duration_ms` 到 absolute deadline 使用 checked 运算；
- 对 I3 已接纳、但平台 `Instant` 不能完整表示的超大合法 duration，钳到从当前 instant 可表示的
  最远未来，不得当作 invalid artifact；
- parent effective deadline 仍由 I4 derive 取较早者，同刻保留 outer owner。

### 3. Internal terminal carrier

增加 eval-internal scope terminal carrier，保留完整
`ExecutionScopeTerminal` / `EffectiveDeadline` owner：

- ancestor cancel 立即归一为 `RuntimeError::Cancelled`；
- local / inherited deadline 保持 internal、不可 ordinary catch、不可 wire/opaque ordinary
  carrier；
- internal terminal经过 diagnostic wrappers仍保持不可 catch；
- generic `ExecutionControlError::BudgetExceeded` 的 instruction limit继续使用既有
  `ExecutionBudgetExceeded`；
- generic deadline error必须重新读取 current scope owner，不能丢成普通可 catch timeout。

本节点不创建最终 `UserException<TimeoutError>`。它只提供精确 owner carrier与“是否由当前
timeout scope拥有”的判定；E4 在拥有 request heap/catch context的位置，只把匹配当前
source/nesting的 local terminal物化为带 correlation/stack 的 user exception。Inherited/request
deadline必须继续穿过当前 timeout/catch。

### 4. Owner-aware checkpoint core

提供统一 helper，顺序固定：

1. scripted/production clock `now`；
2. current scope terminal；
3. shared instruction accounting / budget；
4. generic deadline error再次按 current scope恢复 owner。

helper 要能表达 function entry、loop condition、backedge、lane start/end、tail前和长生成片段的
checkpoint kind/units；本节点只实现 core和单元测试，不改各 evaluator call site。

### 5. Explicit compile bridge

当前 eval crate先于任何 E1 test就被 F445G 的两个 non-exhaustive match阻断。只允许在
`eval_context.rs` 给以下四个新 kind增加明确 fail-closed placeholder：

- `LinkedStmtIr::{Timeout, Concurrent}`
- `LinkedExprIr::{Timeout, ConcurrentValue}`

placeholder 必须返回稳定 `InvalidArtifact`，说明 F445H-E4 尚未接线；不得 wildcard、顺序执行、
丢弃 body/plan或实现部分语义。E4 将替换这四个 arm。不得改已有 `ValueBlock`。

## Test-first 与验收

先新增 RED，至少覆盖：

- context capture child / drop parent restoration；
- `OwnedProgramExecutionContext` round-trip保留 child scope；
- scope unavailable与 nesting derive failure fail closed；
- normal、local、inherited/request、ancestor cancel carrier矩阵；
- nested inner-earlier、outer-earlier、equal deadline outer-only owner；
- internal terminal在 ordinary payload/catch/wire wrapper中均被拒绝；
- instruction limit仍为既有 ordinary budget error；
- generic deadline race恢复 scope owner；
- scripted clock第 N checkpoint越界，且 checkpoint count有界；
- 最大合法 duration safe clamp与普通 duration精确加法；
- compile bridge四个 arm稳定 fail closed。

运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e1-scope-core/build/cargo-target \
  cargo test -p skiff-runtime-eval program_execution_scope -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e1-scope-core/build/cargo-target \
  cargo test -p skiff-runtime-eval scope_terminal -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e1-scope-core/build/cargo-target \
  cargo check -p skiff-runtime-eval --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e1-scope-core/build/cargo-target \
  cargo test -p skiff-runtime-eval --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e1-scope-core/build/cargo-target \
  cargo fmt --check
git diff --check
```

若 full eval有既有失败，独立复现并分类；本节点新增/focused必须全绿。

## 写集与结构

只允许：

- `runtime/eval/src/program_execution.rs`
- `runtime/eval/src/program_execution/**`
- `runtime/eval/src/error.rs`
- `runtime/eval/src/error/**`
- `runtime/eval/src/eval_context.rs`，仅四个 compile-bridge arm及其直接测试
- 本 result

`program_execution.rs` 与 `error.rs` 已分别超过 1400 / 2200 行；新 production/test 责任必须放入
child modules，root只做字段、variant、module与薄转发接线。不得再把大段实现追加到 root。

不得修改 capabilities、request、env、actor、stream、host、native、artifact、compiler、
linked-program或 Router。

worktree：

`/Users/geek/workspace/skiff-p5-f445h-e1-scope-core`

branch：

`codex/p5-f445h-e1-scope-core`

base：`27618e61`，再 cherry-pick 本任务文档。

提交 implementation，再只新增并提交：

`P5-F445H-E1-eval-scope-terminal-checkpoint-core-result.md`

最终 clean。不得派子 Agent、merge/rebase/push、stable/live/network。若 internal carrier导致必须
修改写集外 exhaustive consumer，或 UserException物化必须提前进入 exceptions/catch owner，停止并
精确上报，不做兼容 hack。
