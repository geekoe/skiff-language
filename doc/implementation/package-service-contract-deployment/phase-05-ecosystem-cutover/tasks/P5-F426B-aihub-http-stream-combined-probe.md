# P5-F426B AIHub HTTP stream merged-state combined probe

状态：Ready。只读批次集成探针。

## 直接父节点

- `P5-F426-connect-wire-and-http-consumer-wave.md`

## 输入与职责

精确读取父节点记录的Skiff与Internals commits。唯一允许写入是本leaf result文档；不得修复任何source、
test、fixture或tooling。该probe不是最终验收。

## 必须验证

在隔离临时store/build root中，对同一merged candidate依次执行：

1. AIHub service source/receipt/package-store/workflow guard tests；
2. AIHub browser Node tests、syntax与static server MIME tests；
3. canonical isolated AIHub service graph/type-check，使用父节点Skiff checkout；
4. 若canonical graph通过，生成PackageArtifact、ServiceContract、ServiceDeployment与RuntimeAssembly，
   证明：
   - exactly五个service-call operation；
   - `ServiceProtocolIdentity`相对F425前语义不变；
   - 两条events是raw HTTP server stream；
   - AIHub WebSocket ingress为零；
   - client只使用`POST /v1/chat/events`；
5. legacy反向搜索：AIHub production无`WebSocket`、`chat.request`、`sendTextToConnection`、
   `/ws`、旧unary events handler或compat fallback。

若任何上游命令失败：

- 停止依赖它的后续动态证明；
- 记录首错、全部同一编译批次独立诊断、owner、遮挡范围与最小repair write set；
- 区分F425B/C回归、既有current compiler/source drift与环境问题；
- 返回`COMBINED_FAIL`，不得顺手修复。

## 证据与交付

记录每条命令、真实discovery/pass/fail/skip、临时root、精确input commit/tree与最终clean状态。新增并提交：

`P5-F426B-aihub-http-stream-combined-probe-result.md`

不得merge/rebase/push/stable/live，不访问真实provider。

