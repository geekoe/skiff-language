# Leaf Task: 修复 ActorSelfField 专用 lowering 跳过 self expression key 导致 while 内调用错位降级

## 引用链

- 直接父节点：主 Agent `/root` 派发的 C 波任务信封 `/root/skiff_fix_lowering`。
- 依据/根因摘要：`skiff_blockers_preflight`（信封内授权引用的定位结论，本任务不重述全部证据）。
- 仓库规则：`/Users/geek/workspace/skiff/AGENTS.md`、`/Users/geek/workspace/multi-agent-development.md`。
- baseline：`integration/actor-wave-a` 固定 commit `1532bd7bc8cb55217a52d1fa17c602a74f7aac51`
  （`git rev-parse` 已验证；集成分支后续移动到 `5b740ccc` 合入 router 波，与本任务无关，本任务仍锚定
  信封给定 commit）。`70021ae5`（引入 ActorSelfField 专用 IR）与 `fcbf89ef`（while 全链路）均为其祖先，
  已用 `git merge-base --is-ancestor` 验证。
- worktree：`/Users/geek/workspace/wt-skiff-fix-lowering`，branch `dev/fix-actor-self-lowering`。
- 集成 Agent：`skiff_integration`（集成分支 `integration/actor-wave-a` 唯一写者）。本任务不 merge、不 push、
  不写集成分支。

## 根因（预检确认）

`70021ae5` 在 `compiler/lowering/src/function_lowering.rs` 引入 ActorSelfField 专用 lowering 时，
`self.<field>` 命中后直接返回 `ExprIr::ActorSelfField` / `AssignTargetIr::ActorSelfField`，不再递归
`lower_expr(self)`，因此 `self` 这个 identifier 的 expression key 未被消费。所有 fact（expression type、
`ResolvedCallTargetFacts`）都按 AST 全量 preorder 编号，lowering 每少消费一个 key，后续表达式整体前移一位；
`resolved_call_targets.target(key)` 查不到事实，`lower_call` 落入 fallback：`root.drain.*` 被降成
`CallTargetIr::Builtin { op }`，actor 句柄调用解析失败。while（`fcbf89ef`）之后同一函数体后续表达式
继续错位，使该缺陷在 while 场景可观察。

`suspend_analysis.rs` 与 `type_inference.rs` 的独立 key 遍历均完整递归子表达式，无同类 skip
（预检已核查），不在本任务范围。

实现期在同一调用链发现并闭合第二个缺口（同属任务条款 `self.run()/actor 句柄调用不可解析`，
见 `resolved_local_impl_receiver_call_target`）：`self.run()` 自递归时 builder 侧
`exact_impl_self_edge_target` 只接受 `LocalImplMethod` 事实，actor 的 `self.run` 事实是
`ActorMethod`，故 builder 不产出 exact edge；但 lowering 侧镜像 `exact_impl_self_edge_receiver`
无条件认领，抢先报 `no exact typed source target`，`actor_method_call_target` 无法到达。
最小修复：仅当调用事实是 `LocalImplMethod` 时才认领 exact self edge，否则返回 `Ok(None)`
放行到 actor 调用路径。此修复不改 record impl 的 fail-closed 行为（事实为
`LocalImplMethod` 时原错误路径保留），`ambiguous_generic_impl_receiver_fails_before_file_ir`
等既有测试保持通过。

## 修复面（写入范围，预期 owner，非机械白名单）

1. `compiler/lowering/src/function_lowering.rs` `lower_assign_target` 的 `Expr::Field` 分支
   （ActorSelfField 路径，基线约 917 行）：命中 actor self field 时先消费 `self` object 的
   expression key 再返回。
2. 同文件 `lower_expr_with_expected` 的 `Expr::Field` 分支（ActorSelfField 路径，基线约 1354 行）：
   同样消费 `self` object 的 expression key。
3. 同文件 `resolved_local_impl_receiver_call_target`：仅当调用事实为 `LocalImplMethod` 时
   认领 `exact_impl_self_edge_receiver`，actor 自调用放行到 `actor_method_call_target`。
4. `compiler/lowering/src/source_file_lowering/tests.rs` 新增 actor impl 方法回归测试：方法体内
   先读/写 `self.field`，再在 `while` 条件与体内调用同模块 root 函数（期望 `LocalExecutable`）、
   跨模块 root 函数（期望 `PublicationExecutable`）、`self.run()` 与 actor 句柄方法（期望
   `ActorMethod`），并断言这些调用没有降级为 `CallTargetIr::Builtin`。
5. 本文件 `doc/implementation/actor-self-lowering-fix/leaf-task.md`（叶子任务合同，提交进分支）。

## 硬约束（禁止面）

- 只动 compiler/lowering；不碰 runtime、router、artifact-model、compiler/source、compiler/input 等
  其他 owner 表面；不新增关键字、配置、schema 或第二套实现。
- 不修改 while 之外的行为；除修复 key 消费外不重排无关代码。
- 不与在途 router 波（`dev/actor-get-create-router`，已合流，写集零重叠）、其他修复波竞争文件。
- 不 merge main / 集成分支，不 push；完成后直接交接 `skiff_integration` 并通知主 Agent。

## 聚焦验证（证据 owner：/root/skiff_fix_lowering）

```bash
cd /Users/geek/workspace/wt-skiff-fix-lowering
git diff --check
cargo test -p skiff-compiler-lowering --no-fail-fast
node scripts/verify.mjs --only compiler
node scripts/verify.mjs   # 基线 36/36 保持全绿
```

构建缓存使用共享 `CARGO_TARGET_DIR=/Users/geek/workspace/skiff/target`（任务信封指定，不另建大 target）。

## 自验收矩阵

`设计/任务条款 | 代码证据 file:line 或 symbol | 反向搜索证据 rg | 测试命令`

- ActorSelfField 读路径消费 self key | `lower_expr_with_expected` Field 分支 |
  `rg -n "ActorSelfField" compiler/lowering/src/function_lowering.rs` 仅剩 2 处生产分支且均先消费 key |
  `cargo test -p skiff-compiler-lowering`
- ActorSelfField 写路径消费 self key | `lower_assign_target` Field 分支 | 同上 |
  `cargo test -p skiff-compiler-lowering`
- actor 自调用放行 ActorMethod | `resolved_local_impl_receiver_call_target` 的
  `exact_impl_self_edge_receiver` 分支 | `rg -n "exact_impl_self_edge" compiler/lowering` |
  `cargo test -p skiff-compiler-lowering`
- while 内 root.* / actor 句柄调用不降级 Builtin | 新回归测试断言 |
  `rg -n "Builtin" 新测试范围`（仅负向断言） | `cargo test -p skiff-compiler-lowering`
- 仓库级全绿 | 无 | `git diff --check` | `node scripts/verify.mjs`
