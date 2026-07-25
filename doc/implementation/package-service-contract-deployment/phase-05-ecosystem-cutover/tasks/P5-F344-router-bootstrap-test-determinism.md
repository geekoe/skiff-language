# P5-F344 Router bootstrap test determinism

状态：Ready（test-only）。

## 直接父节点

- Router consumer实现与测试证据：
  `P5-F341-service-error-router-consumer-result.md`
- 当前 C0复验：
  `P5-F339-response-error-schema-reacceptance-result.md`

本任务只关闭 H/R/T合流组合探针发现的测试时序问题，不修改 Router production或 error wire。

## 起点与复现

- 起点 commit：`29bcb43adb6647ed87add4dfcbd2212352b96a4a`
- 起点 tree：`b7bc3697f02f620e4c15ec480feb530189ea628c`
- 以下8文件组合运行时稳定复现119项中的1项失败：
  `assembly-runtime-endpoint.test.ts > keeps the complete generic runtime switch on the composite endpoint`。
- 失败收到`router.bootstrap`却期待`runtime.registered`；单独运行同一 selector为1/1 PASS。
- 原因候选是测试在 WebSocket open后才挂一次性 message listener，初始 bootstrap在不同调度时序下可能
  已丢失或成为下一条消息。不得通过修改 production发送时序、延迟或删除 bootstrap修复。

## 写入边界与目标

唯一允许修改：

- `router/tests/assembly-runtime-endpoint.test.ts`

不得修改 production、其它测试、protocol/corpus、配置/lockfile、设计与父 task/result。

必须：

1. 在该测试或其本文件 helper中显式处理初始`router.bootstrap`，使其无论在 listener建立前后到达都不会
   被误判为 register响应。
2. 不把通用`nextRuntimeFrame`改成无限忽略所有unexpected frame；允许的跳过必须限定为此连接的初始
   bootstrap且至多一次，随后仍对协议顺序 fail closed。
3. 保留本测试对 v2 control `response.error`的既有断言。
4. 不增加 sleep、扩大 timeout或串行化整个 Router suite掩盖竞态。

## 验证

先列出 selector，再：

- 单独运行目标 selector；
- 连续至少5次运行 F341+F342 的同一8文件组合，119/119每次均通过；
- `pnpm --filter @skiff/router run type-check`；
- `git diff --check`。

不得运行完整 workspace/root/stable/live，不 push。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f344-router-bootstrap-test`
- branch：`codex/p5-f344-router-bootstrap-test`
- 新的一次性开发 Agent；
- 新增`P5-F344-router-bootstrap-test-determinism-result.md`，写明精确竞态、修复机制、5次组合证据；
- 提交并返回 commit，不修改 task状态，不承接后续节点。
