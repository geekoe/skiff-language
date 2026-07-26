# P5-F373 Host HTTP gateway resume after generation correction

状态：Ready（恢复同一未完成F365任务；使用新Agent，不复用原会话）。

## 直接父节点

- 原任务：`P5-F365-host-http-gateway-admission-wire.md`
- 暂停证据：`P5-F365-host-http-gateway-admission-wire-blocker.md`
- 已完成前置：`P5-F370-http-gateway-assembly-generation-correction-result.md`

F370 production commit为
`7ddcbf31b06bdc86212b0f406df55f49c06f1231`。它只修改
`runtime/request/src/http_gateway_execution.rs`及新增直接测试；原F365 worktree的未提交inventory没有这两个
path，只有另一个`runtime/request/src/execution_budget.rs`局部helper，因此语义与写入范围不冲突。

## 保留状态与恢复步骤

- worktree：`/Users/geek/workspace/skiff-p5-f365-host-http-gateway`
- branch：`codex/p5-f365-host-http-gateway`
- 当前HEAD仍是原F365 checkpoint；20个tracked path保留Host实现，约
  `1577 insertions / 2039 deletions`。

1. 开始时核对dirty inventory与暂停文档一致；不得checkout、reset、stash或丢弃任何现有F365修改。
2. 为防止长时间未提交成果丢失，先对现有F365 owned diff运行`git diff --check`并提交一个明确标为
   checkpoint的本地commit；该commit不是完成声明。
3. 在checkpoint后引入F370 production commit。优先普通`git cherry-pick 7ddcbf31...`；若发生真实冲突，
   停止并返回精确冲突，不手工复制共享seam或重写F370测试。
4. 继续完成原F365全部条款和验证；特别重跑：
   - 同一assembly连续两个HTTP gateway request；
   - typed/raw unary、raw server stream；
   - timeout clamp、cancel、response ceiling、reload route pin；
   - internal service operation与通用WebSocket generation lifecycle回归。
5. 原F365 production反搜、Host/request/eval checks和非零selector要求保持不变。F370不能替代Host层对route、
   activation identity与generation的独立校验。

## 写入与交付

写入范围仍严格沿用F365。除F370 cherry-pick外，不修改其禁止的共享DTO/loader/linker/eval/transport/Router/
test-runner/service。若继续验证暴露新的共享owner，按工作流再次停止，不吞入Host。

完成时：

- checkpoint之后的Host修复/测试可为一个追加commit；
- result写入原
  `P5-F365-host-http-gateway-admission-wire-result.md`，并记录checkpoint、F370 cherry-pick与最终tree；
- worktree clean，不merge/rebase/push，不操作stable/live；
- 返回原F365完整自验收矩阵，不能只报告generation blocker已消失。

启动5分钟内必须进入状态核对和checkpoint；不得重新从头实现或覆盖已有diff。
