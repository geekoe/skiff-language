# P5-F445H-E4R5C combined reacceptance

状态：Ready。E4R5 初次FAIL后的全新冻结候选独立复验与唯一完整 eval gate owner。PASS后解除
I6前置，不代表F445H/Phase 05整体完成。

## 直接父节点与候选

- `P5-F445H-E4R5-combined-integration-acceptance-result.md`
- `P5-F445H-E4R6-callback-stack-shape-closure-result.md`
- `P5-F445H-E4R7-stream-deadline-test-semantics-closure-result.md`
- `P5-F445H-E4R8-activation-wait-stack-shape-closure-result.md`

冻结 production/tests候选为 integration commit
`bf55ede018526751a2db101a42900c4e07fe08a8`。相对初次 R5候选：

- E4R6只在 callback wait call-site加入private pinned heap indirection；
- E4R7只迁移五条 `cfg(test)` stream deadline tests到 E1/R4语义；
- E4R8只在 activation wait call-site加入private pinned heap indirection；
-没有公共 API、prepared owner、E1/E2/E3、current-scope或业务语义变化。

当前没有在途 production/tests写入。签发本task只新增验收合同。

## 角色与唯一写集

唯一允许写入：

- 新增 `P5-F445H-E4R5C-combined-reacceptance-result.md`

production、tests、fixture、Cargo/manifest/lockfile及其它文档只读。不得修失败、格式化源码、调整
断言或派子 Agent。发现failure立即记录唯一owner。

不得 merge/rebase/push，不运行stable、live、network、MongoDB或其它仓库。

## 必须验证

### 1. 修复点与组合入口

先运行：

- `f445h_e4r7_stream_deadline` listing/execution：5/5；
- `f445h_e4r_combined` listing/execution：5/5。

只读确认：

- callback `prepared.wait(&interpreter)`只有R6 call-site private `Box::pin`；
- activation `operation.wait()`只有R8 call-site private `Box::pin`；
-两者仍经过同一个 `await_actual_pending`，finalize顺序未改；
- production没有 `RUST_MIN_STACK`、test stack配置或公共boxing API；
- E4R7 source diff全位于 `#[cfg(test)] mod tests`，non-test production不变；
-五条新tests仍真实非零，没有ignore/删除/零filter伪证据。

### 2. 唯一完整 eval gate

在默认worker栈、显式清除 `RUST_MIN_STACK` / `RUSTFLAGS` 的同一独立target上运行一次完整
`skiff-runtime-eval` suite，lib使用单线程以提供确定顺序，但不得提高栈：

```bash
env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5c-acceptance/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --no-fail-fast -- --test-threads=1
```

必须取得每个binary合法完整summary，预计inventory至少：

- lib 395；
- `catch_fixture_closure` 4；
- `f445h_e4r_combined` 5；
- `representation_wrap_consumer` 6；
- doc tests 1；
-总计411。

不能用abort前输出、filtered exact或提高栈对照推算。若inventory因合法新增变化，记录实际值并
解释；任何failure/SIGABRT均为FAIL。

完整 suite应包含并实际执行：

- spine 23；
- timeout 11；
- concurrent 11；
- stream 22；
- E4R7 deadline 5；
- combined integration 5。

### 3. Check与反向检查

```bash
CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5c-acceptance/build/cargo-target \
  cargo check -p skiff-runtime-eval --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r5c-acceptance/build/cargo-target \
  cargo fmt --check
git diff --check
```

复核初次R5静态结论在当前候选仍成立：

1. 四个 evaluator fail-closed diagnostics为零；
2. production静态 `maySuspend`/binding/effect不决定segment释放；
3.没有旧pre-suspend helper、yield、sequential concurrent fallback；
4. timeout internal carrier不进入ordinary payload/wire；
5. stream current-scope无request-root/generic token fallback；
6. natural End唯一disarm，异常仅本地cleanup initiation；
7. DB O6 owner唯一、DbQuery同步；
8. R6/R8只改变private future布局，R7 production diff零；
9.没有新增公共 API、栈环境依赖或compatibility路径。

命中若属于test/ABI metadata/合法E3 owner，按路径分类，不只报总数。

### 4. 证据映射

result沿用初次R5已冻结的T05–T12与九组actual-Pending映射，但必须以本次完整suite合法summary
重新签发执行证据。特别记录：

- callback Ready/Pending已在默认栈执行；
- activation Ready/Pending与service-error consumer已在默认栈执行；
-五条request deadline carrier/raw cancellation语义已执行；
- combined R1–R4五条仍通过。

## Verdict

只有以下全部成立才可写：

```text
PASS
E4R_COMPLETE = YES
I6_UNBLOCKED = YES
```

- deadline 5/5、combined 5/5；
-完整eval每个binary与doc test全部通过、无abort；
- locked check、fmt、diff通过；
-静态反向检查无生产残留/重复owner/fallback；
-候选期间无在途写入。

若失败，写 `FAIL` 并给精确命令、test/path/call chain、唯一owner和失效证据；不得修候选。若需要
公共语义/owner扩张，另标 `TASK_SCOPE_EXPANDED`。

result记录commit/tree、所有实际数量、blocking/non-blocking、warning/ignored分类、未运行的
live/Mongo/stress残余风险及clean状态。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r5c-acceptance
branch   codex/p5-f445h-e4r5c-acceptance
```

只提交result；返回result commit、verdict、完整计数、blocking/non-blocking和clean worktree。
不得 merge、rebase或 push。

风险：高。此Agent是独立验收，不承担实现。
