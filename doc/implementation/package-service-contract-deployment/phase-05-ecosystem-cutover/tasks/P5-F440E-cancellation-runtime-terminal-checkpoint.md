# P5-F440E Cancellation runtime terminal checkpoint

状态：Ready。确定性实现leaf。

## 直接父节点

- `P5-F440-external-manifest-and-bidirectional-websocket-batch.md`
- `P5-F440C-cancellation-compiler-artifact-hard-cut-result.md`
- `P5-F439A-cancellation-public-surface-owner-audit-result.md`

本任务实现F439A DAG中的R0。精确上游集成checkpoint：

| Commit | Tree |
| --- | --- |
| `adc846fcc102ab23ffdd461066e72459ed9f9cee` | `affa74598b94f41aa058e2be8d11ec0037fe5a83` |

## 目标与写集

把`runtime/capability-context`中的cancellation收敛为内部execution terminal：它仍能唤醒并终止token、
blocked stream和child work，但不能产生ordinary wire payload、catch identity、service envelope或
`CancelError`。Deadline仍是可观察的`TimeoutError`。

唯一production/test写集：

- `runtime/capability-context/**`

另可新增本leaf result。禁止修改native、eval、request、host、transport、model、router、scripts、std、
compiler/artifact或权威设计。不得派子agent。

## 实现合同

1. 测试先行，覆盖：
   - `ExecutionControlError::Cancelled`；
   - `ExecutionBudgetReason::Cancelled`经budget carrier；
   - `StreamRuntimeError::Cancelled`；
   - already-cancelled token、pending suspension、blocked stream send/next和outer/inner cancellation。
2. 上述cancellation carrier只能提供明确的内部terminal classification/查询，不能实现或返回：
   - `RuntimeErrorPayload { code:"CancelError" }`；
   - platform catch projection/identity；
   - ordinary response/service serialization事实。
3. 保持token wake、stream cleanup、waiter removal、single-terminal和lifetime release；不能用“删除错误分支”
   让future永远pending。
4. `DeadlineExceeded`与instruction/operation budget的既有非取消分支继续精确投影为
   `TimeoutError`；cancel/deadline同时ready时现有上层biased语义所需分类不能丢失。
5. 若crate需要为下游提供迁移API，只能暴露编码无关的internal terminal classification；不得添加新的公开
   cancellation名义类型、字符串code或serializer。
6. 最终反向搜索：本写集production无`CancelError`和
   `PlatformBuiltinErrorIdentity::Cancel`；`Cancelled`/`CancellationToken`保留且逐一分类。

## 验证与停止规则

- 精确列出并运行execution control、token、stream cancellation focused tests。
- 运行`cargo check -p skiff-runtime-capability-context`（按实际package名核对）及fmt/diff。
- 可只读记录下游compile break，但不得修改下游；R1会消费本checkpoint。
- 不运行完整verify、Router、live、instance或stable。
- 如果无法在本crate建立internal terminal而必须先改runtime/model公共enum或native/eval production，
  立即返回`TASK_SCOPE_EXPANDED`，不要越界。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f440e-cancellation-terminal`
- branch：`codex/p5-f440e-cancellation-terminal`
- result：`P5-F440E-cancellation-runtime-terminal-checkpoint-result.md`

实现与result分别提交；返回commit/tree、测试计数、consumer blocker、reverse search和clean状态。
不merge/rebase/push。
