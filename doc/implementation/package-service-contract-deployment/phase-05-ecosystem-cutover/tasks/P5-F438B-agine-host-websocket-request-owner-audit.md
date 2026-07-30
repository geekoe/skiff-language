# P5-F438B Agine / Host WebSocket request owner audit

状态：Ready。高风险、只读consumer路径闭合审计。

## 直接父节点

- `P5-F438-websocket-outbound-request-response-batch.md`

父节点已取代F436B对Host方向未冻结的结论：Host是外部peer，可以接收Skiff通过WebSocket发起的请求并
返回平台response；它不能借此向Skiff发起业务request。启动时只读本任务，需要依据时沿引用链读取。

## 精确输入

| Repo | Commit | Tree |
| --- | --- | --- |
| Skiff设计 | `64a0ab4ec85d25899dc8563ac6d647edad8ed23e` | `562adcfc8baa595969a4dd1ccd2e67c4053814b9` |
| Internals | `066b5135a8e06f87acfd614e408e05b35453f4eb` | `23be114f0d4b838eff1c7b214a40fc9c57cdd354` |

本leaf只审计Internals production/test owner并把result写入Skiff任务目录；不修改任一生产仓。

## 只读范围

允许读取：

- `agine/service/**`
- `agine/host/**`
- `agine/client/**`
- `shared-client/**`
- 直接相关的Internals package/API类型与workflow tests
- Skiff父节点与以下精确权威文档：
  - `doc/architecture/package-service-contract-deployment.md`
  - `doc/architecture/gateway-runtime-adapter-boundary.md`
  - `doc/reference/std-surface.md`
  - `doc/reference/runtime.md`
  - `doc/reference/api-yml.md`

禁止修改production、test、manifest、design或generated fixture。唯一写入是本leaf result。

## 必须回答

1. 从所有旧WebSocket `eventName` / `requestId` / receive dispatcher入口建立完整矩阵，至少覆盖：
   - Host activation/hello、presence、reconnect与capability announcement；
   - Host tool execution/result/cancel；
   - Host file list/search/current-directory及其browser桥接；
   - 浏览器chat/provider/thread等普通请求；
   - server notification与stream delivery。
2. 对每条路径按终态分类，不能按文件名猜：
   - external peer主动发起，必须迁到HTTP；
   - Skiff向Host发起并等待，迁到平台`requestJsonToConnection`；
   - Skiff单向notification，保留`send*`；
   - 真正跨request持久业务生命周期，保留明确命名的`toolCallId`、`attemptId`、`jobId`等；
   - 完全dead legacy graph，删除。
3. 对可迁移为平台req/res的每个操作列：
   - Skiff caller、精确Host connection来源与授权检查；
   - request/response concrete type、method名owner和错误投影；
   - Host handler、取消入口、并发与乱序响应；
   - deadline/disconnect/reconnect语义；
   - 哪些现有DB pending/relay record、cleanup loop和业务`requestId`可删除。
4. 特别判断Host file browser当前“browser request → service relay → Host → service receive → browser
   response”能否在一个HTTP request内挂起等待Host response。列出HTTP timeout、Host timeout、取消与
   browser disconnect的owner；不得因旧实现异步就默认保留`jobId`/polling。
5. 审计`EnhancedWebSocket.request`及其它client/host自建correlation：
   - 哪些方向已被HTTP替代；
   - 哪些应由新的Host peer responder取代；
   - 是否还存在合法的非Skiff WebSocket RPC consumer；
   - 删除或保留的精确production/test owner。
6. 冻结Host peer protocol adapter边界：
   - 只接受固定platform request/cancel envelope；
   - 必须原样回显request id；
   - method dispatch、payload validation和fixed success/error response；
   - unknown method、malformed payload、cancel race与handler throw；
   - 不能发送client-initiated platform request。
7. 列出connect-only Agine收敛的精确owner：
   `service.yml`、connect handler、active connection持久化/actor更新、legacy receive/API exports、
   config与tests。区分可在Skiff平台checkpoint前完成的节点和必须等待后的节点。
8. 给出互斥后继DAG、精确写集、聚焦验证、combined风险探针与反向搜索allowlist。若某个业务操作的
   同步/持久生命周期仍无法从代码和权威设计确定，返回`TASK_NOT_EXECUTABLE`并列出最小用户决策。

## 证据

只运行源码反搜、静态Node test listing或必要的便宜只读命令；不运行service build/type-check、浏览器、
stable/live、真实Host或完整canonical workflow。Result必须给出文件:符号级producer/consumer证据，
不能只列匹配计数。

## Worktree与交付

- Skiff worktree：`/Users/geek/workspace/skiff-p5-f438b-agine-host-request-audit`
- Internals worktree：`/Users/geek/workspace/internals-p5-f438b-agine-host-request-audit`
- 分支：`codex/p5-f438b-agine-host-request-audit`

新增并提交`P5-F438B-agine-host-websocket-request-owner-audit-result.md`。返回commit/tree、完整分类矩阵、
后继DAG、未决问题与两个clean状态。不得修改production/test、merge、rebase、push、stable/live；完成后
不得自行承接implementation。
