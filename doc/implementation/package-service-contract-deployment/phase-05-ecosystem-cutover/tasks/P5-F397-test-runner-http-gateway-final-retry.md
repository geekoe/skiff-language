# P5-F397 Test-runner HTTP gateway final retry

状态：Ready。

## 直接父节点

- `P5-F396-test-runner-http-gateway-final-revalidation-blocker.md`
- 完整验收仍以`P5-F396-test-runner-http-gateway-final-revalidation.md`为准。

## Worktree与前置

- `/Users/geek/workspace/skiff-p5-f386-package-test-http-gateway`
- branch `codex/p5-f386-package-test-http-gateway`
- clean HEAD `71687e3765fc302611aad5de22a095d1621e4b8f`

开始时确认F387 owned diff没有`router/**`。依序cherry-pick：

1. F390 fixture/tests：
   `53c79dc6e029137d7e0ba987f8ba5f5fb0de480f`
2. F392 current filesystem loader：
   `e4cf24313717ec8842bf0e4771cc130746e2af34`
3. F394 runtime protocol v4：
   `540f93c4fa52885bd8498a9144dd1b6dea49ec29`

任一步仍有真实冲突才停止；不得跳过F390、降级identity或dual-read。

随后完整执行F396：

- provider `echo`显式`serviceCall: true`与1-operation direct proof；
- 两个test-runner clippy owner的结构性修复，不加allow；
- T1/T2 unit/integration/bins/clippy/Node v2 receipt；
- 真实isolated package-test、strict control、inline setup及package service dependency；
- 临时进程/端口/目录清理。

写`P5-F397-test-runner-http-gateway-final-retry-result.md`，追加本地commit，worktree clean；不
merge/rebase/push，不操作stable/live，不派子Agent。新production owner则
`TASK_SCOPE_EXPANDED`。
