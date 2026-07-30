# P5-F394 Router runtime protocol current service identity

状态：Ready。

## 直接父节点

- `P5-F387-test-runner-http-gateway-convergence-blocker.md`
- `P5-F392-router-current-artifact-generations-result.md`

F392已把filesystem snapshot owners迁到`skiff-service-protocol-v4`；本节点关闭runtime wire/protocol parser
剩余的v3门禁，不改变消息字段或WebSocket业务语义。

## Worktree

- `/Users/geek/workspace/skiff-p5-f394-router-runtime-protocol-v4`
- branch `codex/p5-f394-router-runtime-protocol-v4`
- base：包含F392与本任务的Skiff phase-05 integration。

## 必须完成

1. `router/src/protocol/runtimeProtocol.ts`及其直接canonical parser/validator只接受
   `skiff-service-protocol-v4`，删除v3，不dual-read。
2. 更新所有direct runtime protocol fixtures/tests/goldens到current v4；不得仅在F387测试中词法替换。
3. 确认assembly activation、runtime connection registration和request dispatch对同一exact
   ServiceProtocolIdentity逐值传递；Router不重算identity。
4. scoped production反搜`skiff-service-protocol-v3`归零；若其它明确archived fixture命中，逐一迁移或
   报告独立owner。

## 写入边界

允许：

- `router/src/protocol/runtimeProtocol.ts`
- 其direct Router protocol/activation/connection tests与fixtures。

禁止：

- snapshot/filesystem owners（F392已完成）；
- protocol字段shape、HTTP/WS gateway语义；
- Host/runtime Rust、test-runner；
- stable/live。

## 验收

运行runtime protocol、assembly activation、runtime connection和相邻request dispatch非零测试，Router
scoped typecheck及`git diff --check`。至少用current v4 exact object通过parser，并证明v3被拒绝。

写`P5-F394-router-runtime-protocol-current-service-identity-result.md`，production/tests/result本地
commit，worktree clean；不merge/rebase/push，不派子Agent。若需要改变wire字段而非generation，返回
`TASK_SCOPE_EXPANDED`。
