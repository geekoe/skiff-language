# P5-F439A 取消公开面与内部终止owner审计

状态：Ready。高风险、只读语义owner审计。

## 直接父节点

- `P5-F439-websocket-jsonrpc-and-cancellation-batch.md`

父节点记录已冻结语义、精确输入、当前遮挡和后续DAG。启动时只读本任务；需要依据时沿引用链向上读取。

## 输入与目的

| commit | tree |
| --- | --- |
| `aacee2129934a6aebc2975293b5b4ed4b209c42f` | `617021923ad3d7072d19deecb9f41460dd2163e4` |

权威设计要求ancestor cancellation不可被用户catch，且不存在`CancelError` public type。当前代码仍在
compiler prelude、runtime error carrier、router协议和测试中使用该名字。本审计要区分：

1. 必须删除的用户可name/catch/throw surface；
2. 必须保留但应改为内部终止状态或控制frame的实现；
3. 历史result文字和不影响production的fixture。

它不实现、不修改production/test/design、不运行完整gate。

## 只读范围

允许读取`std/**`、`compiler/**`、`artifact-model/**`、`runtime/**`、`router/**`、
`test-runner/**`、`cross-system-fixtures/**`、`scripts/**`及父节点引用的权威文档。唯一允许写入是本
leaf result。禁止修改Internals、skiff-packages、stable/live。

## 必须回答

1. 列出`CancelError`全部production definition、registration、serialization、matching、catch lowering、
   runtime materialization、router projection和直接tests；区分symbol spelling、wire code与内部enum。
2. 画出以下真实取消链并标明每个owner：

   ```text
   ancestor/request cancel或losing concurrent lane
     -> execution control
     -> suspension/host operation abort
     -> pending cleanup
     -> work item终止
     -> response/telemetry
   ```

3. 明确哪些位置不得再生成普通throw envelope、不得进入service error serialization、不得被
   `catch<E>`匹配；同时说明timeout为何仍生成`TimeoutError`。
4. 核对service-to-service、gateway request、stream consumer break、actor lane、native host operation、
   runtime disconnect等入口是否共享同一内部terminal owner，避免只修WebSocket路径。
5. 冻结最小实现DAG和互斥写集。若移除public type必须先落compiler/artifact共享checkpoint，再列runtime、
   router与fixture follower；不要把跨owner修改塞进一个叶子任务。
6. 为每个节点给出首次实际修改、聚焦测试、反向搜索和最早风险探针；说明哪些昂贵gate留给最终owner。
7. 若设计仍不足以决定“不可捕获取消”如何映射某个真实production入口，返回
   `TASK_NOT_EXECUTABLE`并给出精确路径和最小决策，不自行发明可捕获替代错误。

## 证据与交付

使用`rg`、源码调用链、Cargo metadata和test listing；最多运行确认selector非零的便宜聚焦测试。不得运行
完整Rust/Router suite、live、instance或stable。

- worktree：`/Users/geek/workspace/skiff-p5-f439a-cancellation-audit`
- 分支：`codex/p5-f439a-cancellation-audit`
- result：`P5-F439A-cancellation-public-surface-owner-audit-result.md`

新增并提交result，返回commit/tree、owner矩阵、实现DAG、验证矩阵与clean状态。完成后不得自行承接实现。

