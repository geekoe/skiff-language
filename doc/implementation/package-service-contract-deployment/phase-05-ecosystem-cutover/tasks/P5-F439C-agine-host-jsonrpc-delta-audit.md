# P5-F439C Agine / Host JSON-RPC协议差量审计

状态：Ready。中风险、只读consumer delta审计。

## 直接父节点

- `P5-F439-websocket-jsonrpc-and-cancellation-batch.md`

父节点记录新权威语义和精确输入。F438B result已拥有业务分类、旧consumer图和Host同步读取范围；本任务只
审计从旧自定义wire迁到`jsonrpc-2.0-text`后的差量，不重新做全仓泛化review。

## 输入与目的

| Repo | Commit | Tree |
| --- | --- | --- |
| Skiff design | `aacee2129934a6aebc2975293b5b4ed4b209c42f` | `617021923ad3d7072d19deecb9f41460dd2163e4` |
| Internals | `faa11b188c570ca763f107ddd829d52b8fe8861f` | `140d3a03851b64d513fd97c5860e713b8fc314de` |

审计`host.files.list`、`host.files.search`、`host.current-directory`三项平台RPC的Host peer、Skiff caller、
共享protocol fixture和取消/错误owner，使后继任务无需从F438B过时wire自行推断。

## 只读范围

允许读取：

- Skiff父节点、F438B result及其直接引用的权威设计
- `/Users/geek/workspace/internals-phase-05-integration/agine/protocol/**`
- `/Users/geek/workspace/internals-phase-05-integration/agine/host/**`
- `/Users/geek/workspace/internals-phase-05-integration/agine/service/**`中三项同步Host读取及旧relay
- `/Users/geek/workspace/internals-phase-05-integration/agine/client/**`中对应HTTP caller
- `/Users/geek/workspace/internals-phase-05-integration/shared-client/**`中仅被上述路径直接触及的代码

唯一允许写入是本leaf result。禁止修改production/test/design、stable/live。

## 必须回答

1. 以F438B的三项canonical method为起点，列出当前Host TypeScript handler、service caller、request/
   response types和tests；确认JSON-RPC `params`均为object，业务代码不需要transport `id`。
2. 冻结Host peer adapter的最小职责：
   - 解析单个JSON-RPC 2.0 text request，拒绝batch和非法shape；
   - 按method调用既有typed HostService；
   - success/error返回并原样回显string id；
   - 接收`$/cancelRequest`并用内部AbortController终止仍在执行的可取消读取；
   - notification、未知method、异常和取消竞态的精确处理。
3. 把旧string `code/detail` error设计改成JSON-RPC integer `code/message/data`，给出Host内部异常到有限
   integer code的owner与脱敏边界；禁止发明`platform.*`字符串。
4. 核对断线、late result、duplicate id、同connection乱序和取消后完成时，Host adapter如何只写一次response；
   业务方不得感知id或自行维护pending map。
5. 核对Agine service的`requestJsonToConnection`调用点如何恢复typed result并处理
   `WebSocketRequestError`封闭分支、`TimeoutError`和不可捕获取消；删除旧DB/browser/Host relay的写集仍
   必须符合F438B分类。
6. 给出最小consumer实现DAG、互斥写集、聚焦测试与cross-language fixture owner。明确哪些任务必须等Skiff
   shared checkpoint，哪些纯Host adapter/test可提前实现。
7. 若现有Host框架无法在不新增公共业务语义的情况下支持取消或typed dispatch，返回
   `TASK_SCOPE_EXPANDED`并列精确证据，不扩张成新Host框架设计。

## 证据与交付

使用`rg`、源码追踪和test listing；不运行完整build、browser、canonical workflow、live或stable。

- worktree：`/Users/geek/workspace/skiff-p5-f439c-agine-host-delta-audit`
- 分支：`codex/p5-f439c-agine-host-delta-audit`
- result：`P5-F439C-agine-host-jsonrpc-delta-audit-result.md`

新增并提交result，返回commit/tree、delta owner矩阵、实现DAG、验证矩阵与clean状态。完成后不得自行承接
实现。

