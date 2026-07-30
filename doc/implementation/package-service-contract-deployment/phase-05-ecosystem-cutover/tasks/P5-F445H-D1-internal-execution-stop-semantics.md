# P5-F445H-D1 Internal execution stop semantics

状态：Ready。用户已决定不把“取消请求”作为Skiff公开能力；保留的只是runtime内部停止无用执行的
生命周期信号。本节点先更新权威设计，再重发DB evaluator任务。

## 直接父节点

- `P5-F445H-O5D-db-terminal-lifecycle-owner-preflight-result.md`

用户确认的设计方向：

1. 不提供按request id取消、用户源码cancel/catch/inspection或public `CancelError`；
2. deadline、并发loser、stream consumer退出、connection owner结束、drain和lease-lost仍可触发
   runtime内部停止；
3. timeout对外仍是`TimeoutError`，internal stop本身没有业务error/payload；
4. client/peer断开表示没有response consumer，不承诺撤销已发生的业务副作用；
5. transaction尚未commit时尽力abort；commit尝试一旦开始，不切换为“保证abort”，其结果可能
   完成或unknown，late result丢弃；
6. lease异常停止时先停止续租、尽力release，TTL是最终回收保证；
7. normal success/business-error路径仍严格等待transaction/lease正常terminal；
8. abnormal stop cleanup为有界best-effort，不要求可见request结果等待cleanup ack，也不要求
   exactly-once completion；
9. internal transport stop frame可作为节省资源的hint保留，但不属于公开协议或正确性前置；
10. 第一版WebSocket JSON-RPC不提供peer `$/cancelRequest`或`-32800 Request cancelled`。

## 权威文档目标

只修改：

- `doc/reference/runtime.md`
- `doc/reference/db.md`
- `doc/reference/service-yml.md`
- `doc/reference/std-surface.md`
- `doc/architecture/gateway-runtime-adapter-boundary.md`
- `doc/architecture/package-service-contract-deployment.md`
- `doc/architecture/open-issues.md`
- 本result

要求：

- 统一使用“internal stop / 内部停止”描述runtime lifecycle，不把它写成用户可请求的cancel；
- 清楚区分visible timeout/error、transport consumer gone与内部停止；
- 明确late value/error/heap write丢弃，但外部副作用不回滚；
- 删除generic host operation必须声明commit point、cancel-safety、cleanup action的过强当前要求；
- effect metadata继续保留effect kind、target/conflict key、idempotency与concurrency语义；
- HTTP/stream disconnect可以触发内部停止hint，但不产生cancel response或rollback承诺；
- WebSocket JSON-RPC删除peer cancel notification、cancel code与outbound best-effort peer cancel；
- internal `request.cancel` / `connection.request.cancel` frame若在架构清单中保留，必须标成可选
  stop hint和late-result隔离，不是public cancellation；
- DB transaction/lease正常路径与abnormal stop路径分别写清；
- `db lease`的request id只作诊断，不再承诺未来控制面按request取消；
- 不修改historical task/result或implementation plan，避免重写已经发生的审计记录。

## 非目标

- 不在本节点修改production/tests、wire或fixture；
- 不删除内部`CancellationToken`/`Cancelled`代码；
- 不实现新的cleanup grace config；
- 不决定durable queue item取消语义；
- 不运行Cargo、stable/live/network/Mongo。

## 验收

反向搜索当前权威文档，确认：

- 无public `$/cancelRequest`、`-32800 Request cancelled`或按request取消能力；
- 无generic required `cancel-safety` / `commit point` / `cleanup action` metadata；
- `request.cancel` / `connection.request.cancel`若存在，只以internal optional stop hint出现；
- timeout、stream、concurrent、transaction、lease仍有完整可执行语义；
- Markdown fence与`git diff --check`通过。

结果文件：

- `P5-F445H-D1-internal-execution-stop-semantics-result.md`

完成权威文档并验收后，单独重发简化O6；不得继续执行O5D的L1/L2/L3 structured cleanup DAG。
