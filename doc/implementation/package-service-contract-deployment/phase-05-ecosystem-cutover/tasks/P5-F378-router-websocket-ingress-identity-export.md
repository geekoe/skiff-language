# P5-F378 Router WebSocket ingress identity export

状态：Ready。

## 直接父节点

- `P5-F375-registry-generation-revalidation-result.md`

父节点已在隔离Router启动中证明production import/export断链：

```text
router/src/gateway/assemblyWebSocketGateway.ts
  imports canonicalAssemblyWebSocketIngressIdentity
  from ../router/assemblyRuntimeRegistry.js

router/src/router/assemblyRuntimeRegistry.ts
  does not export or define that symbol
```

本节点只恢复canonical helper的单一owner与Router启动，不改变WebSocket业务消息路由设计。

## Worktree与范围

- worktree：`/Users/geek/workspace/skiff-p5-f378-router-websocket-identity-export`
- branch：`codex/p5-f378-router-websocket-identity-export`
- base：包含本任务的Skiff phase-05 integration。

开始时先沿直接import和现有identity helper确认canonical定义应位于registry owner还是共享identity模块。
必须复用现有assembly WebSocket ingress canonical字段与哈希规则；不得新造第二套算法。

允许修改：

- `router/src/gateway/assemblyWebSocketGateway.ts`
- `router/src/router/assemblyRuntimeRegistry.ts`
- 已存在的相邻canonical identity helper模块；
- 直接Router type/startup/WebSocket assembly测试。

禁止：

- assembly/artifact DTO或identity schema；
- WebSocket selector/envelope/业务消息路由语义；
- HTTP gateway、Host/runtime/test-runner；
- skiff-packages、Internals、stable/live。

若仓库中没有足够事实确定唯一算法，或修复需要协议字段变化，返回`TASK_SCOPE_EXPANDED`。

## 验收

至少证明：

1. production import/export闭合，helper只有一个canonical定义；
2. Router TypeScript typecheck通过；
3. 直接import/startup probe不再抛ESM missing export；
4. WebSocket assembly gateway聚焦测试与相邻runtime assembly registry测试非零通过；
5. HTTP gateway测试无回归；
6. reverse search没有悬空同名import或重复实现。

写`P5-F378-router-websocket-ingress-identity-export-result.md`，production/tests/result一个或两个清晰本地
commit，worktree clean；不merge/rebase/push，不操作stable/live。新Agent执行，不派子Agent。
