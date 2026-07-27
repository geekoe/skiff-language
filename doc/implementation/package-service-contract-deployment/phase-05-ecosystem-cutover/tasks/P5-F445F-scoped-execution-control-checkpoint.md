# P5-F445F Scoped execution control checkpoint

状态：Ready。F445B-I4 implementation node；可与F445E并行。

## 直接父节点

- `P5-F445B-timeout-expression-implementation-preflight-result.md`
- `P5-F445D-timeout-syntax-checkpoint-result.md`

## 输入

Skiff integration：

`/Users/geek/workspace/skiff-phase-05-integration` @ `128129ff`

本节点不消费source AST，只实现runtime通用scoped control primitive。

## 完成目标

只在 request/capability-context边界建立I5/I6可复用的通用模型：

1. `EffectiveDeadline`明确拥有monotonic absolute instant、source和nesting；derived scope只能取
   parent/request/local中最早deadline，不能延长parent。
2. 相同absolute deadline按reference选择outer source；deadline source/site/nesting稳定可诊断。
3. request instruction statistics/limit、poll accounting和trace facts由child scope共享；
   local timeout不得调用request-wide `record_deadline_exceeded`或污染first-failure telemetry。
4. parent/ancestor cancel与local-scope cancellation分源：
   - ancestor cancel始终是不可捕获terminal；
   - local deadline固定为可投影 `TimeoutError`的scope terminal；
   - local winner取消child work但不取消shared parent token；
   - scope退出/被catch后parent execution仍可继续。
5. 提供有界timer/wait/lease生命周期：normal、timeout、ancestor cancel和drop均零泄漏；
   同时ready保持当前cancel-first terminal invariant，nested同deadline只outer可观察。
6. API只表达execution control；不加入source/eval/WebSocket/HTTP特殊字段，不修改wire或artifact schema。
7. 为I5冻结安装/恢复current scope所需contract，为I6冻结operation invocation时读取effective control的
   contract；本节点不修改这些consumer。

## Test-first

先在两个crate建立paused/fake-clock RED，覆盖F445B T07–T10的primitive部分：

- local earlier、parent/request earlier、same deadline outer；
- nested source/nesting；
- normal/drop恢复parent；
- local timeout后parent继续；
- ancestor cancel不可转timeout；
- cancel/deadline同ready cancel-first；
- instruction/poll共享但local telemetry隔离；
- timer/waiter/lease零泄漏和late completion无复活。

运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445f-scoped-control/build/cargo-target \
  cargo test -p skiff-runtime-request --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445f-scoped-control/build/cargo-target \
  cargo test -p skiff-runtime-capability-context --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445f-scoped-control/build/cargo-target \
  cargo fmt --check
git diff --check
```

## 写集与提交

只允许：

- `runtime/request/**`
- `runtime/capability-context/**`
- 本任务result

worktree：

`/Users/geek/workspace/skiff-p5-f445f-scoped-control`

branch：

`codex/p5-f445f-scoped-control`

先提交implementation，再只新增并提交：

`P5-F445F-scoped-execution-control-checkpoint-result.md`

最终clean。不得派子Agent、merge/rebase/push、stable/live/network。若正确分源必须改eval/host，
只输出精确I5/I6 handoff，不在本层越界。
