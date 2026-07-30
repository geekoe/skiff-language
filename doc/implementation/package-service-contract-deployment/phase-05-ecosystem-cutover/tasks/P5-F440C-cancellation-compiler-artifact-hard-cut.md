# P5-F440C Cancellation compiler/artifact hard cut

状态：Ready。确定性实现leaf。

## 直接父节点

- `P5-F440-external-manifest-and-bidirectional-websocket-batch.md`
- `P5-F439A-cancellation-public-surface-owner-audit-result.md`

F439A result的C0节点是本任务完整合同。需要细节时只沿这两个父节点引用读取。

## 目标与写集

删除所有用户可name/throw/catch的`CancelError` compiler builtin，并在artifact admission最早checkpoint
硬拒绝旧`TypeRefIr::Builtin("CancelError")`，包括throw/catch/nested union等位置。`TimeoutError`和内部
`request.cancel`不变。

唯一production/test写集：

- `compiler/**`
- `artifact-model/**`

另可新增本leaf result。禁止修改runtime、router、scripts、std、其它task/result或权威设计。不得派子agent。

## 实现合同

1. 先增加或改成真实failing tests，证明：
   - 短名`CancelError`和qualified `std.error.CancelError`均不能解析；
   - 不能用作constructor/type/throw/catch/rethrow或union leaf；
   - 手写legacy File IR无论出现在普通type ref、Throw payload、Catch catch type或nested union都在
     linker conversion之前的artifact validation失败。
2. 从compiler builtin registry删除name/symbol/kind，不新增cancellation特例或隐藏alias。
3. Artifact validator按canonical builtin registry硬拒绝legacy spelling；不得悄悄lower成unknown/native。
4. 保持`TimeoutError`注册、lowering和现有正例byte-equivalent。
5. 反向搜索production：不再注册或发射`CancelError`；只允许明确命名的negative rejection test。

## 验证与停止规则

- 精确列出并执行compiler builtin spelling tests。
- 精确列出并执行artifact type-ref validation tests。
- 运行受影响crate `cargo check`与fmt/diff check。
- 不运行完整verify、Router、live、instance或stable。
- 如果实现必须修改runtime/linker production、scripts或公共artifact schema generation，立即停止并返回
  `TASK_SCOPE_EXPANDED`；不要越界修复。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f440c-cancel-compiler-artifact`
- branch：`codex/p5-f440c-cancel-compiler-artifact`
- result：`P5-F440C-cancellation-compiler-artifact-hard-cut-result.md`

实现与result分别提交；返回commit/tree、测试计数、反向搜索和clean状态。不merge/rebase/push。
