# Leaf Task: F5a chat smoke E2a 回归修复（无 create 的 keyed actor dispatch 提交误判）

## 引用链

- 权威 chat smoke（Gate 定位）：`chat/send` 立即报
  `actor activation has create input but no create declaration`，agent run 不启动；
  调用链 `dispatchThreadActorTick → std.actor.get<ThreadActor> + dispatch actor.tick()`；
  `ThreadActor` 是 `key(id)` 无 create 声明的 keyed actor。
- E2a 提交侧契约：`doc/implementation/dispatch-e2a-actor-submit-leaf.md`
  （snapshot 冻结；create-less actor 的 createInput 为空数组 bytes；
  recoverable gate 只在 create 输入存在时执行）。
- E2b 执行侧契约：`doc/implementation/dispatch-e2b-actor-execute-leaf.md`
  （router 在 entry 缺失时以 snapshot createInput put-if-absent 保存最小 entry，
  执行侧按 linked create 声明解码；`[]` 是 create-less 激活的合法 bootstrap）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、`/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`ee12eb53`（`main` HEAD，`git rev-parse` 确认）。
- worktree：`/Users/geek/workspace/skiff-chat-regression-fix`，branch
  `chat-regression-fix`。
- 集成 Agent：`/root/chat_fix_integration`；主 Agent：`/root`。本任务不 merge、
  不 push、不写共享集成分支；共享主 worktree 只读。

## 根因（零 worktree 只读预检证据）

- `runtime/native/src/dispatch/actor.rs`（`ActorNativeDispatch::prepare`）：
  `std.actor.get` 的 create args 是 `args.iter().skip(1)` 的规范化数组。无 create
  参数的调用产生空数组，`canonical_json_bytes(&Value::Array([]))` 得到 2 字节
  `[]`，作为 bootstrap payload 传往 actor instance。
- `runtime/eval/src/actor_instance.rs`（`materialize_instance`）：`create_input:
  request.bootstrap_payload.to_vec()` 原样冻结这 2 字节；`[]` 对无 create 声明
  的 keyed actor 是合法激活输入（`create_args.len() == 0`）。
- `runtime/eval/src/task_ops.rs`（E2a `actor_activation_snapshot`）：
  `if !create_input.is_empty()` 用**字节长度**判定“有 create input”。`[]` 为 2
  字节，被误判为有 create input；随后
  `create.ok_or_else(|| InvalidArtifact("actor activation has create input but
  no create declaration"))` 直接 definite rejection，任何
  `task.submit.request` 都不会产生。这就是 chat smoke 的精确症状。
- 测试缺口：`runtime/eval/src/task_ops/tests/actor_submit.rs` 夹具
  `linked_fixture` / `ActorSubmitFixture::new` 全部硬编码
  `create: Some(...)` + `bootstrap_payload: ["account-1"]`，没有覆盖
  “无 create 声明 + `[]` bootstrap + dispatch 方法”的正例；native
  `std.actor.get` 的 prepared 测试也没有断言无 create args 时的 bootstrap。

## 设计决策（最小正确修复，不改设计语义）

方案：**语义空判定 + 计划级 has_create 标记**，落在 E2a 提交侧的判定点。

```rust
// A create-less keyed actor is activated with the canonical empty array `[]`
// (2 bytes). That is not a create input: only an actor with a linked create
// declaration can have one, so the recoverable gate must not run for it.
if create.is_some() && !create_input.is_empty() {
    gate_create_input(...)
}
```

决策理由（对比过的备选）：

- **不修正序列化侧**（把无 create args 的 `[]` 改成 0 字节 payload）：
  E2b 执行侧 `materialize_instance` 仍要求 bootstrap 是合法 JSON 数组
  （`serde_json::from_slice` + `as_array()`），router entry 恢复/冷激活路径
  依赖 `[]`；0 字节会让 create-less 激活在 branch-3 snapshot 恢复时新增
  `CreationInputsDecode` 回归。进一步证据：
  `router/src/task/admission.rs` 的 `entry.create_input.is_empty()` 是刻意的
  legacy-entry 判别（0 字节 = 未冻结创建输入 → 回退 snapshot；`[]` = 已冻结的
  create-less 创建输入 → 直接用作 bootstrap）；把 `[]` 改成 0 字节会让 branch 2
  把所有 create-less entry 误判为 legacy，每次回退 snapshot 恢复。序列化侧改动
  需要同时改 materialize 与 router admission 语义，写集与风险都更大，且没有把
  “有 create 输入”的判定收敛到 declaration 事实。
- **语义空判定只解析数组**（`[]` 视为空输入）：对 reachable 状态与
  `create.is_some() && !create_input.is_empty()` 行为等价（带 create 且 0 参数
  的激活输入只能是 `[]`，gate 0==0 恒过），但需要额外解析一次 JSON，且对
  “有 create 声明才是 create 输入”的表达不如计划级标记直接。
- 因此采用 **has_create（`declaration.create.is_some()`）与字节非空联合判定**：
  - 无 create 声明 → 永不 gate（本 bug 修复）；
  - 有 create 声明 → 行为与 E2a 合并时完全一致（非空即 gate；`[]` 对 0 参数
    create 也走 gate 并恒过，语义不变）；
  - snapshot 的 `create_input` base64 保持 `[]`（`W10=`）不变，E2b 的
    entry 恢复 / 执行解码语义不变。

反事实（若不做此修复）：

- chat smoke 继续在提交前拒绝所有无 create keyed actor 的方法 dispatch；
  agent run 永不启动。
- 任何把无 create keyed actor 用作任务接收者的合法服务都会暴露同一错误；
  wire / corpus / router 侧无法在提交前识别，错误只出现在运行期。

## 测试（含回归用例）

1. eval 层 `actor_submit` 夹具支持无 create：`linked_fixture` /
   `ActorSubmitFixture` 参数化 `has_create`；无 create 时
   `declaration.create = None`、`bootstrap_payload = []`。
2. `actor_method_submit_keyed_actor_without_create_submits_once`（有 actor
   execution frame）：无 create + `dispatch actor.method(...)` 正例，断言恰好
   一次提交、snapshot `create_input == base64("[]")`、expected plan 字段为空。
3. `actor_method_submit_keyed_actor_without_create_external_context_submits_once`
   （F0b 外部上下文 + actor instance store）：复现 chat 形态（store 中
   `[]` bootstrap 的 keyed actor，HTTP 侧直接 dispatch），断言同上。
4. `std.actor.get` 直接调用正例（native prepared 测试）：id-only 调用产生
   `[]` bootstrap payload，getOrCreate 正常返回 ActorRef。
5. 有 create 的 actor dispatch 正例回归：既有
   `actor_method_submit_freezes_snapshot_and_submits_once` /
   `actor_method_submit_external_context_freezes_snapshot_and_submits_once`
   保持不变并全绿（含 `["account-1"]` gate 断言）。

## 实际写集

```text
doc/implementation/dispatch-f5a-chat-regression-leaf.md   # 本叶子文件
runtime/eval/src/task_ops.rs                             # create.is_some() && !is_empty() 判定
runtime/eval/src/task_ops/tests/actor_submit.rs           # 夹具参数化 + 无 create 正例 2 例
runtime/native/src/dispatch/tests/prepared/actor.rs       # getOrCreate 记录 bootstrap + 无 create 正例
```

未改：`doc/reference/`、`doc/architecture/`、`doc/implementation/**` 既有文件；
`runtime/native/src/dispatch/actor.rs` 序列化语义（`[]` 保持 2 字节）；
`actor_instance.rs` materialize 语义；E2b router 执行侧；共享主 worktree。

## 自验收矩阵（实际证据）

| 任务条款 | 代码证据 | 测试命令 |
| --- | --- | --- |
| 无 create keyed actor 不得被当作有 create input | `task_ops.rs` `actor_activation_snapshot`：`create.is_some() && !create_input.is_empty()` | `cargo test -p skiff-runtime-eval --lib task_ops::tests::actor_submit` |
| keyed actor 无 create + dispatch actor.method 提交正例 | `actor_submit.rs` 无 create 夹具 + 正例（frame / external-store 两形态） | 同上 |
| std.actor.get 直接调用正例（`[]` bootstrap） | `prepared/actor.rs` bootstrap 记录 + id-only 正例 | `cargo test -p skiff-runtime-native dispatch::actor` |
| 有 create 的 actor dispatch 正例回归 | 既有 `actor_method_submit_freezes_snapshot_and_submits_once` 等保持 | 同上（eval actor_submit 全套） |
| 相关 runtime eval/host/native 测试全绿 | 聚焦测试结果 | 见验证记录 |
| cargo check 受影响 crates | eval / native / host | 见验证记录 |

## 验证记录

- `cargo test -p skiff-runtime-eval --lib task_ops::tests::actor_submit`：9/9
  PASS（7 既有 + 2 新增无 create 正例）。
- `cargo test -p skiff-runtime-native --lib actor_get_route`：2/2 PASS（既有 +
  新增 id-only bootstrap 正例）；`dispatch::` 全套 44 PASS。
- `cargo test -p skiff-runtime-eval --lib`：473 PASS。
- `cargo test -p skiff-runtime-host --lib`：429 PASS。
- `cargo test -p skiff-runtime-native --lib`：126 PASS。
- `cargo check`（eval / native / host 及依赖 crate）：PASS。
- `cargo fmt --check`（eval / native）PASS；`git diff --check` PASS。
- 回归有效性：临时 stash 掉 `task_ops.rs` 修复行后，2 个无 create 正例
  在 `keyed_actor_without_create` 过滤下 FAILED（0 passed, 2 failed，提交点
  panic）；恢复修复后同过滤 2/2 PASS。两个测试确实复现并锁定原 bug。
- `git diff --check` PASS。

## 交接注意事项

- 修复只改 E2a 提交侧判定；E2b 执行侧（router owner lane / materialize）本就接受
  `[]`，无需改动，本次未触碰。
- 无 create keyed actor 的 snapshot `createInput` 在 wire 上仍是 `W10=`（`[]`），
  corpus / 既有 wire 语义不变。
