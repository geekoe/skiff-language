# P5-F445H-O6R9 DB/Actor fixture case closure result

状态：`PASS / FIXTURE_CASE_SURFACE_REFROZEN / O6R7_O6R8_UNBLOCKED`。

fixture + GREEN 提交为 `5725efea`。production prerequisite 仍为 `2d5df5ae`，测试代码证据锚定
`549476b9`；工作树启动 HEAD `6851674b` 相对该锚点只增加本叶子任务文件。此结果只重新冻结共享
test-only fixture，解除 O6R7 transaction 与 O6R8 lease 矩阵的 fixture prerequisite，不声明任一矩阵
或 combined acceptance 已通过。

## 1. 实现结果

唯一 linked program/executable 在原位完成机械扩展：

- canonical lease claim 的 `binding_slot` 从 `None` 改为 slot `0`，同一 executable 增加 index `0` 的
  local slot，`frame_size = 1`；
- 原 `entry` 与 `empty` block 保持不变；
- 新增 crate-test-only `BODY_CREATE_BLOCK_LABEL`，其 block 只引用一个
  `LinkedStmtIr::Expr`，该 statement 指向 executable 中既有 raw create
  `LinkedExprIr::DbOperation`；
- 新增 crate-test-only `ILLEGAL_FLOW_BLOCK_LABEL`，其 block 只引用一个真实
  `LinkedStmtIr::Return`，可由 transaction/claim child 选择以产生禁止 flow；
- `db_actor_fixture_checkpoint` 增加 slot/frame、block→statement→expression、非法 flow 与单一
  program/executable 的结构断言；原 raw/prepared/Actor smoke 继续执行。

没有新增第二份 `LinkedFileUnit`、`EvalRuntimeProgram`、`LinkedExecutable`、fake、Actor frame 或
evaluator seam。没有修改 transaction、lease、ordinary child 或 production。

## 2. RED → GREEN

未修改 fixture 的基线先运行两个精确 selector，均发现 1 个测试并按预期失败：

| Selector | RED 结果 | 精确原因 |
| --- | --- | --- |
| `db_actor_transaction_fixture_exposes_explicit_illegal_flow_case` | 0/1，FAILED | 只有 `entry`/`empty` 两个空 block，且 statements 为空 |
| `db_actor_lease_fixture_exposes_required_binding_variant` | 0/1，FAILED | canonical claim 的 `binding_slot == None` |

最小 fixture 修改后，两个 selector 原样重跑，均为 1/1 GREEN。RED 未删除、忽略或放宽。

## 3. 聚焦验证

以下结果均锚定 fixture commit `5725efea` 的代码状态：

| 命令 / selector | 实际结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval db_actor_transaction_fixture_exposes_explicit_illegal_flow_case -- --nocapture` | PASS，1/1 |
| `cargo test -p skiff-runtime-eval db_actor_lease_fixture_exposes_required_binding_variant -- --nocapture` | PASS，1/1 |
| `cargo test -p skiff-runtime-eval program_db::tests::fixture::db_actor_fixture_checkpoint -- --nocapture` | PASS，1/1 |
| `cargo test -p skiff-runtime-eval db_actor_transaction_explicit_body_actual_pending_releases_actor_segment -- --nocapture` | PASS，1/1 |
| `cargo test -p skiff-runtime-eval db_actor_lease_claim_pending_uses_one_actor_segment -- --nocapture` | PASS，1/1 |
| `cargo test -p skiff-runtime-eval program_db::tests::ordinary:: -- --nocapture` | PASS，12/12 |
| `cargo check -p skiff-runtime-eval --tests --locked` | PASS；仅仓库既有 warning |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

所有 Cargo 命令均使用任务指定的
`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r9-fixture-cases/build/cargo-target`。
lease 的 O6R6 regression 在 claim 返回 `None`、调用方使用空 `Env` 的条件下仍通过，证明无 handle 时
不会尝试导入新增 binding slot。

## 4. 反向检查

在 `runtime/eval/src/program_db/tests` 中：

```text
EvalRuntimeProgram::new                         1 处（fixture/program.rs）
executables: vec![LinkedExecutable             1 处（fixture/program.rs）
```

两个新增 label 常量均为 `pub(in crate::program_db::tests)`；fixture 位于
`runtime/eval/src/program_db.rs` 的 `#[cfg(test)] mod tests` 下，因此没有 production 可见性。fixture
commit 的变更文件精确为：

```text
runtime/eval/src/program_db/tests/fixture/program.rs
runtime/eval/src/program_db/tests/fixture.rs
```

本 result 是唯一额外写入。Cargo、manifest、lockfile、fixture state/store/actor、production、
Actor E3、capability-context、service-db 与 driver tests 均无变更。

## 5. 自验收矩阵

| 任务条款 | 代码 / 反向证据 | 验证 | 结果 |
| --- | --- | --- | --- |
| 非空 binding slot 与有效 layout | claim slot `0`；layout 含 index `0`，frame size `1` | checkpoint 结构断言 | PASS |
| claim `None` 不导入 binding | canonical claim 已有 binding，但 O6R6 `None` 回归仍使用空 `Env` 成功 | lease regression 1/1 | PASS |
| 保留 `entry` / `empty` | 原两个 block 未改 | diff + checkpoint | PASS |
| statement-backed body create | label 常量指向 block→`LinkedStmtIr::Expr`→既有 raw create expression | checkpoint 结构断言 | PASS |
| 真实禁止 flow | label 常量指向 block→`LinkedStmtIr::Return` | checkpoint 结构断言 | PASS |
| child 不复制 magic label/program | 两个 crate-test-only label 常量为单一 API | 可见性与 ownership 反向搜索 | PASS |
| 单一 program/executable owner | 各 owner 构造搜索只有 1 处；program 只有一个 service file、无 package | checkpoint + `rg` | PASS |
| 原 smoke 与 O6R6 不回退 | raw/prepared/Actor checkpoint、两个 O6R6 selector | 1/1、1/1、1/1 | PASS |
| ordinary 不回退 | 非零 ordinary selector | 12/12 | PASS |
| RED 原样转 GREEN | 两个 selector 先各 0/1 RED，后各 1/1 GREEN | 命令记录 | PASS |
| 唯一写集 | fixture commit 仅两份允许 fixture 文件；本文单独提交 | `git show` / `git status` | PASS |
| 静态质量 | locked check、fmt、diff check | 全部通过 | PASS |
| 环境与集成边界 | 未运行完整 eval/stage gate、stable、live、network 或 MongoDB；未 merge/rebase/push | 命令记录 | PASS |

## 6. 未决问题

本节点无未决 fixture 问题。O6R7/O6R8 仍须在各自独立节点完成 transaction/lease 行为矩阵；fixture、
DB/Actor production 或相关依赖若变化，本结果证据失效。
