# P5-F445H-O6R9 DB/Actor fixture case closure

状态：Ready。O6R7 与 O6R8 在第一次必需变体上都证明原“fixture API frozen”检查点不完整：
transaction 没有 statement-backed 非法控制流 block，lease claim 固定
`binding_slot = None`，也没有 claim body 的 DB-operation/非法-flow block。本节点只修正这个共享
test-only fixture owner，一次补齐两个已冻结矩阵所需的机械 case surface。

## 直接父节点

- `P5-F445H-O6R7-db-actor-transaction-matrix-resume-result.md`
- `P5-F445H-O6R8-db-actor-lease-matrix-resume-result.md`
- `P5-F445H-O6R2-db-actor-shared-fixture-checkpoint-result.md`

父节点沿 O6R1、O6R 与 D1 引用链追溯到唯一权威设计。production prerequisite 仍为
`2d5df5ae`；本任务起点 `549476b9` 已集成两个 test-only RED：

- `db_actor_transaction_fixture_exposes_explicit_illegal_flow_case`
- `db_actor_lease_fixture_exposes_required_binding_variant`

两个 RED 当前各自与 O6R6 最小 GREEN 同 selector 共存。不得删除、忽略或放宽 RED。

## DAG 位置与已确认 owner

这是 O6R7/O6R8 的新共享 fixture prerequisite。完成后只重新冻结测试夹具，并解除两个矩阵重启；
不改变 production 行为，也不构成 transaction、lease 或 combined acceptance PASS。当前候选仍是实现
检查点。

已确认：

- 唯一 linked program owner 是
  `runtime/eval/src/program_db/tests/fixture/program.rs`；
- transaction child 可以 clone `explicit_transaction` 后选择 fixture 已存在的 body label；
- lease child 可以 clone `claim` 后选择 fixture 已存在的 body label；claim 返回 `None` 时不会尝试
  binding，claim success 时才通过 `binding_slot` 导入值；
- `Env::for_program_executable` 依赖 executable 的有效 `SlotLayoutIr`，因此非空 binding slot 必须有
  对应 frame slot；
- body DB case 必须通过真实 block statement 执行 fixture 已有的 linked raw create expression，
  从而由 fake store 的 `BodyCreate` script 驱动；
- 非法-flow case 必须通过真实 `LinkedStmtIr` 产生 `Flow::Return`、`Break` 或等价禁止 flow，不能用
  不存在的 block、缺失 result expression 或手工返回错误代替；
- state/store/Actor frame/linked executable/fake 已足够；本节点不增加第二份 program、fake 或
  evaluator seam。

## 唯一写集

- `runtime/eval/src/program_db/tests/fixture/program.rs`
- `runtime/eval/src/program_db/tests/fixture.rs`
- `P5-F445H-O6R9-db-actor-fixture-case-closure-result.md`

不得修改 transaction/lease/ordinary child、fixture state/store/actor、production、Actor E3、
capability-context、service-db、driver tests、Cargo、manifest 或 lockfile。

## 必须实现

在现有唯一 `LinkedExecutable` 中做最小机械扩展：

1. 为 canonical lease claim 提供非空 binding slot，并为该 slot 提供有效 executable
   `SlotLayoutIr`；保留 claim `None` 路径的语义——没有 handle 时不导入 binding；
2. 保留现有 `entry` 与 `empty` block；
3. 增加一个有 statement 引用的 body-create block，其 statement 通过真实
   `LinkedStmtIr::Expr` 执行现有 raw create linked expression，供 transaction/claim body
   actual-Pending/error case 复用；
4. 增加一个有 statement 引用的非法-flow block，其 statement 真实产生 transaction 与 claim
   都禁止的 flow；
5. 以 crate-test-only 常量或同等单一 API 暴露两个 block label，使 child 不复制 magic label、
   statement 或 linked program；
6. `fixture.rs` 的现有唯一 checkpoint smoke 增加结构断言，证明：
   - binding slot 非空且落在 frame size 内；
   - body-create block 引用的 statement 是指向现有 DB operation expression 的
     `LinkedStmtIr::Expr`；
   - illegal-flow block 引用的 statement 确实产生禁止 flow；
   - 仍只有一个 linked executable/program，原 raw/prepared/Actor 四项 smoke 保持通过。

不得为每个 case 复制 `LinkedFileUnit` 或 executable。不得添加 production API、test-support feature
或 child 专用 fake。若以上机械 surface 仍不能在唯一写集内实现，立即返回
`TASK_SCOPE_EXPANDED`，不得扩大写集。

## RED→GREEN 与验证

先在未修改 fixture 的起点确认两个精确 RED 各发现 1 个测试并失败；随后最小修改 fixture，使两个测试
原样 GREEN。再运行共享受影响 selector：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r9-fixture-cases/build/cargo-target \
  cargo test -p skiff-runtime-eval \
    db_actor_transaction_fixture_exposes_explicit_illegal_flow_case -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r9-fixture-cases/build/cargo-target \
  cargo test -p skiff-runtime-eval \
    db_actor_lease_fixture_exposes_required_binding_variant -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r9-fixture-cases/build/cargo-target \
  cargo test -p skiff-runtime-eval \
    program_db::tests::fixture::db_actor_fixture_checkpoint -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r9-fixture-cases/build/cargo-target \
  cargo test -p skiff-runtime-eval \
    db_actor_transaction_explicit_body_actual_pending_releases_actor_segment -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r9-fixture-cases/build/cargo-target \
  cargo test -p skiff-runtime-eval \
    db_actor_lease_claim_pending_uses_one_actor_segment -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r9-fixture-cases/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::ordinary:: -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r9-fixture-cases/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r9-fixture-cases/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试数：两个 RED selector、fixture smoke、两个 O6R6 回归都必须各为 1/1；ordinary 必须
非零且全绿。反向检查只存在一个 fixture `EvalRuntimeProgram`/`LinkedExecutable` owner，新增 block
和 slot 没有 production 可见性。

不运行完整 eval/stage gate、stable、live、network 或 MongoDB。不得 merge、rebase 或 push。

## 执行与证据边界

风险：中（共享 test fixture，解除两个高风险矩阵）。从启动到第一次修改
`fixture/program.rs` 不超过五分钟；此前只允许确认两个 RED，不重做设计或开放式扫描。不得派子
Agent。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6r9-fixture-cases
branch   codex/p5-f445h-o6r9-fixture-cases
```

先提交 fixture+GREEN，再单独提交 result；返回两个 commit、变更摘要、未决问题和自验收矩阵。
worktree 必须 clean。证据锚定起点 `549476b9` 与 production `2d5df5ae`；fixture program、
DB/Actor production 或相关依赖变化会使证据失效。
