# P5-F445H-E4R5 combined integration acceptance

状态：Ready。E4R 冻结候选的独立验收与唯一完整 eval gate owner。PASS 后只解除 I6 前置，
不代表 F445H、F445 或 Phase 05 完成。

## 直接父节点与冻结候选

- `P5-F445H-E4R1-evaluator-spine-actual-pending-checkpoint-result.md`
- `P5-F445H-E4R2-timeout-catch-owner-closure-result.md`
- `P5-F445H-E4R3-concurrent-actor-evaluator-closure-result.md`
- `P5-F445H-E4R4-current-scope-stream-activation-closure-result.md`
- `P5-F445H-E4R4S-activation-prepared-module-closure-result.md`
- `P5-F445H-E4R5A-combined-red-authoring-result.md`
- `P5-F445H-E4R5AS-combined-test-structure-closure-result.md`

验收 production/tests候选冻结为 integration commit
`da49c17cb6e3c479ea649b936aab8614d3beface`。签发本 task只新增验收合同，不改变
production/tests。候选包含：

- R1：evaluator spine、checkpoint、八组 actual-Pending；
- R2：timeout statement/expression、owner materialization、catch；
- R3：concurrent statement/value与 Actor bridge；
- R4：activation第九组、current-scope stream wait与cleanup；
- R4S：等价 module结构闭合；
- R1S/R5AS：test-only等价结构修正；
- R5A：先在 R1-only base证明 `1 GREEN / 4 RED` 的 combined matrix。

本 Agent必须独立检查当前 production路径与证据，不依赖开发 Agent结论直接给 PASS。

## 角色、写集与禁止面

唯一允许写入：

- 新增 `P5-F445H-E4R5-combined-integration-acceptance-result.md`

production、tests、fixture、Cargo/manifest/lockfile及其它文档全部只读。不得修失败、调整断言、
格式化源码或顺手清 warning；发现 blocker立即停止 verdict并记录唯一 owner。

不得派子 Agent，不得 merge/rebase/push，不运行 stable、live、network、MongoDB或其它仓库。

## 验收覆盖

### 1. Combined真实入口

确认 `f445h_e4r_combined` 精确包含至少5条并全部进入真实 evaluator/Actor/stream入口，而非 private
child helper。运行 listing与execution，必须为5/5：

- R1 actual-Pending Ready/Pending + checkpoint；
- R2 timeout statement/expression；
- R3 concurrent statement/value + Actor；
- R4 activation first-Ready不释放；
- R4 stream current child scope + 非-End cleanup。

对照 R5A RED result，确认原四个失败是在对应 production实现后转绿，而不是测试被放宽、ignored、
改名或绕过。

### 2. 完整 eval gate

完整 `skiff-runtime-eval` suite在当前冻结候选上只运行一次，必须覆盖并记录实际 inventory：

- `f445h_e4r_spine` 至少23；
- `f445h_e4r_timeout` 至少11；
- `f445h_e4r_concurrent` 至少11；
- `f445h_e4r_stream` 至少22；
- `f445h_e4r_combined` 至少5；
- 既有 unit、integration与doc tests。

零匹配 binary不计测试数。记录每个 binary的 passed/failed/ignored/filtered和总数。

### 3. 语义与结构反向检查

对非测试 production做精确搜索和必要只读检查，必须确认：

1. 四个 `F445H-E4 evaluator integration is required` timeout/concurrent diagnostics为零；
2. production `native_call_suspends` 为零；
3. `eval_context`及其 actual-Pending consumer没有旧
   `suspend_actor_segment` / `resume_actor_segment` 预释放 helper；
4. `maySuspend` / `may_suspend`、binding name或effect summary不参与 segment释放；
5. 没有语言级 yield、Tokio `yield_now`、`nosuspend`或 sequential concurrent fallback；
6. timeout只由精确 wrapper owner物化，internal carrier不进入 ordinary payload/wire；
7. current-scope stream没有 request-root / generic request-token fallback；
8. natural End是唯一 disarm；非-End仅承诺本地cleanup initiation一次，异常无远端ack保证；
9. DB O6 wait adapter仍为唯一 owner，operation/transaction/lease未被 E4R复制；
10. `DbQuery`保持同步、不进入 external wait；
11. R2/R3/R4没有回改 `eval_context.rs` root、E1/E2/E3/O1–O6公共 owner；
12. 长 root新增责任已进入 child；R1 combined/actual-Pending测试结构修正没有 production diff。

搜索命中若属于测试、ABI metadata或合法 E3 owner，必须按路径分类说明，不能只报总数或误判为
production blocker。

### 4. T05–T12结果映射

result必须把完整 gate中的实际测试/路径映射到：

- T05：timeout statement/expression、child scope、parent恢复；
- T06：owner materialization、inner catch miss、outer catch hit、catch后继续；
- T07：inner-earlier、outer-earlier、equal outer-only；
- T08：inherited/request deadline不延长、不物化、不可 ordinary catch；
- T09：ancestor cancel优先与scope lifecycle归零；
- T10：纯 CPU loop、generated/literal chunk与instruction accounting；
- T11：dependency、tail、source-order、outer priority、Actor Ready/Pending；
- T12：winner、running loser、late result、outer恢复、stream End/非-End/drop/current child scope；
- 九组 actual-Pending、WebSocket/serverStream/DbQuery同步例外。

不要求逐条重跑focused selector；完整 suite和combined gate已覆盖时，引用实际 test inventory即可。

## 唯一 gate命令

使用独立 target：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5-acceptance/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5-acceptance/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5-acceptance/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5-acceptance/build/cargo-target \
  cargo check -p skiff-runtime-eval --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5-acceptance/build/cargo-target \
  cargo fmt --check
git diff --check
```

不要再单独运行 R1/R2/R3/R4 focused selector；它们已经包含在完整 suite，这里避免重复昂贵证据。

## Verdict

只有同时满足以下条件才可写 `PASS / E4R_COMPLETE / I6_UNBLOCKED`：

- combined 5/5；
- 完整 eval所有实际测试通过；
- locked check、fmt、diff通过；
- 反向检查没有 production残留、重复 owner或fallback；
- T05–T12及actual-Pending/sync-exception证据覆盖完整；
- 当前候选没有在途或未合流写入。

若任何测试或结构检查失败，写 `FAIL`，报告精确命令、路径、调用链、唯一责任叶子及哪些证据
失效；不得修改候选。若失败暴露公共 owner/设计扩张，写 `TASK_SCOPE_EXPANDED`。

result还必须记录：

- 验收代码commit/tree；
-所有命令实际数量和退出结果；
- blocking与non-blocking分开；
- warning/ignored test分类；
-未运行的live/Mongo/长压测残余风险；
-worktree clean状态。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r5-acceptance
branch   codex/p5-f445h-e4r5-acceptance
```

只提交 result；返回 result commit、verdict、完整计数、blocking/non-blocking与clean worktree。
不得 merge、rebase或 push。

风险：高。此 Agent是独立验收与gate owner，不承担实现。
