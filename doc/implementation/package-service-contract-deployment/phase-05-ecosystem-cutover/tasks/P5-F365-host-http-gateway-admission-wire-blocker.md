# P5-F365 Host HTTP gateway admission/wire blocker

状态：Paused；Host主体实现保留在未提交worktree，等待F370修正F363共享seam。

## Exact evidence

- Host task base：`ac98e3057ca5a16434d92430b0356e3451d91ab3`
- Host worktree：`/Users/geek/workspace/skiff-p5-f365-host-http-gateway`
- branch：`codex/p5-f365-host-http-gateway`
- 当前20个tracked path有未提交实现，`1577 insertions / 2039 deletions`；不得清理、覆盖或让其它任务写
  `runtime/host/**`。

F365真实连续请求测试确认：

- `runtime/request/src/http_gateway_execution.rs::validate_request`把canonical
  `routing.assemblyGeneration`与`target.eval().request_activation().generation()`比较；
- 后者是Host process内每次request递增的请求序号，不是pinned activation的assembly generation；
- 第一请求因两者恰为`1`而通过，第二个同一assembly的合法请求因请求序号变为`2`而被错误拒绝为
  `std.service.ProtocolError`；
- exact assembly generation已经存在于
  `target.eval().activation_context().identity().assembly_generation`，且Host admission/wire还会独立核验
  route与activation identity。

该问题属于F363 `runtime/request`共享执行seam，不应由F365越界修改。F370修复并合流后，必须使用新的开发
Agent在保留的Host worktree中引入该精确提交、继续原F365验证；不得复用已停止会话。
