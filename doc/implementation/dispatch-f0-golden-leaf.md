# Leaf Task: F0 golden/fixture 收尾（artifact-model 期望表 + syntax Phase 0 parser golden）

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`（dispatch 阶段 E/F 的最终
  语义事实源；本叶子只做机械同步，不改语义）。
- 用户面契约：`doc/reference/dispatch.md` §3（`std.task.TaskRef` /
  `std.task.status` / `std.task.cancel` 用户面）。
- 批次父节点：`doc/implementation/dispatch-e-batch.md`（F0a/F0b 行已 merged；
  集成 Agent `/root/dispatch_e_integration`）。
- 发现来源：`doc/implementation/dispatch-f0a-pattern-leaf.md`「基线既有失败」节，
  明确记录两个 E1 遗留 fixture/golden 失败，本叶子为 F0 收尾节点。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、
  `/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`c9cc1d3195c98f448abbc7b7fdd1645d95f278a1`
  （`dispatch-e-integration` HEAD，已 `git rev-parse` 验证）。
- worktree：`/Users/geek/workspace/skiff-f0-golden`，branch `golden-fix`。
- 集成 Agent：`/root/dispatch_e_integration`；主 Agent：`/root`。本任务不
  merge、不 push、不写共享集成分支；共享主 worktree 只读。

## 任务合同摘要

F0 收尾两个 E1 遗留的 golden/fixture 失败（均为机械同步，非设计改动）：

1. artifact-model `native_callable_semantics_registry_is_sparse_exact_and_safe`
   测试期望表未包含 `std.task.status` / `std.task.cancel`（E1 已把两条目写进
   `STD_NATIVE_SIGNATURES` / `STD_NATIVE_CALLABLE_SEMANTICS`，测试期望未同步）。
2. syntax Phase 0 parser golden（`fixture-parse-output-baseline.txt`）未随 E1
   新增 `std/task.skiff`（及 E3a 新增
   `test-runner/fixtures/durable-task-e2e-live/main.skiff`）重新生成。

交付：

1. 预检定位失败的确切测试与期望文件、golden 生成方式（仓库既有机制）。
2. 用仓库既有机制更新 artifact-model 测试期望表与 syntax Phase 0 golden。
3. 验证：`cargo test -p skiff-artifact-model`、syntax Phase 0 相关测试全绿；
   受影响 crates `cargo check`；确认写集只含 fixture/golden/期望表 + 本叶子。
4. 本叶子文档记录写集与证据。

禁止：不改生产代码语义；不 push；不动共享主 worktree；不改 `doc/reference/` 与
`doc/architecture/` 既有文档；不改 `doc/implementation/**` 既有文件（本叶子文件
除外）。

## 预检结论（只读，锚定 c9cc1d31）

- artifact-model 生产表已含任务条目：`artifact-model/src/native_signature.rs`
  `STD_NATIVE_CALLABLE_SEMANTICS` 第 125-126 行
  `detached_native("std.task.status", true)` /
  `detached_native("std.task.cancel", true)`；`STD_NATIVE_SIGNATURES`
  第 849-861 行两条 `NativeSignatureDef`（target/binding_key 同名，参数
  `Builtin("TaskRef")`，返回 `Builtin("TaskStatus")` /
  `Builtin("TaskCancelResult")`）。compiler/runtime 侧的 E1 同步已合并。
- 失败期望表：
  `artifact-model/src/native_signature/tests.rs`
  `native_callable_semantics_registry_is_sparse_exact_and_safe`：
  - `expected` BTreeSet 缺 `std.task.status` / `std.task.cancel`；
  - `may_suspend` 的 `matches!` 列表同样缺这两条（生产语义为
    `detached_native(..., true)`，即 `may_suspend = true`）。
- syntax Phase 0 golden：`syntax/src/parser/tests/data/
  fixture-parse-output-baseline.txt` 当前 76 行，而 c9cc1d31 提交的 `.skiff`
  文件共 78 个；`comm` 对差两个条目：`std/task.skiff`（E1）与
  `test-runner/fixtures/durable-task-e2e-live/main.skiff`（E3a）。
- golden 生成方式（仓库既有机制）：`syntax/src/parser/tests/parse_output_carrier.rs`
  `fixture_parse_output_matches_phase0_baseline` 在环境变量
  `UPDATE_PARSER_PHASE0_BASELINE=1` 时把
  `fixture_baseline_entries(repo_root())`（`git ls-files --cached '*.skiff'`
  + `parse_source` 序列化）写回 baseline 文件；不手写 trick。

## 关键实现决策

- artifact-model：只更新测试期望表。BTreeSet 增加 `"std.task.status"` /
  `"std.task.cancel"`；`may_suspend` matches! 列表增加同两条目，与生产
  `detached_native(..., true)` 一致。不新增签名条目、不改生产表。
- syntax：运行既有再生成机制（`UPDATE_PARSER_PHASE0_BASELINE=1`），一次补两个
  缺失条目（76 → 78），不手写 JSON。
- 反事实：若只更新 BTreeSet 而不更新 `may_suspend` 列表，sparse-exact-and-safe
  测试仍会失败（may_suspend 断言 false ≠ 生产 true）；若跳过 golden 再生成，
  每次 syntax 测试都会报 Phase 0 baseline diff。

## 实际写集（commit 后与交接报告一致）

```text
artifact-model/src/native_signature/tests.rs            # 期望表 + may_suspend 列表 +2 条目
syntax/src/parser/tests/data/fixture-parse-output-baseline.txt  # Phase 0 golden 76 → 78 条目
doc/implementation/dispatch-f0-golden-leaf.md           # 本叶子
```

## 自验收矩阵（提交后与交接报告一致）

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| artifact-model 期望表含 std.task.status/cancel | `tests.rs` `expected` BTreeSet + `may_suspend` matches! 列表含两条目；生产表 `native_signature.rs` 已有对应条目 | `git diff --name-only` 不涉及生产 `native_signature.rs`；`rg "std.task" artifact-model` 只有生产表 + 测试期望 | `cargo test -p skiff-artifact-model`（全过） |
| syntax Phase 0 golden 与 78 个提交 .skiff 文件一致 | `fixture-parse-output-baseline.txt` 78 行，含 `std/task.skiff` 与 `test-runner/fixtures/durable-task-e2e-live/main.skiff` | 无手写 baseline；`rg -l "UPDATE_PARSER_PHASE0_BASELINE" syntax` 仅 `parse_output_carrier.rs` | `UPDATE_PARSER_PHASE0_BASELINE=1 cargo test -p skiff-syntax fixture_parse_output_matches_phase0_baseline`（再生成，1 PASS）+ 无 env 重跑同测试（1 PASS）+ `cargo test -p skiff-syntax --lib`（Phase 0 全绿） |
| 受影响 crates 编译 | 写集仅 test/fixture，无生产代码 diff | `git diff --stat` 确认 | `cargo check -p skiff-artifact-model -p skiff-syntax`（PASS） |
| 写集边界 | 仅 3 个文件（本叶子 + 2 fixture/golden/期望） | `git diff --check` PASS；`git status` 无无关文件 | `git diff --check` |

## 停止条件

- 生成机制暴露代码问题（生产语义或生成器缺陷）：不原地猜测，停止并报告主 Agent
  `/root`。
- 需要改变公共契约/架构语义：不原地猜测设计，停止上报。

## 交接

完成后把 branch、worktree 路径、commit/tree、实际写集和自验收矩阵直接报告给
`/root/dispatch_e_integration`，并通知主 Agent `/root`。
