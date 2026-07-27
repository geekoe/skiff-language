# P5-F445H-D1 Internal execution stop semantics result

状态：`DESIGN_UPDATED / INTERNAL_STOP_ONLY`。

本节点只修改权威设计和任务记录，没有修改production或tests。Skiff第一版不再提供公开的
“取消请求”能力；deadline、并发loser、stream consumer退出、connection owner结束、drain和lease
lost仍可让runtime内部停止已经没有价值的执行。

## 1. 冻结结论

1. 用户源码没有cancel API、按request id取消API、`CancelError`或stop状态检查API。
2. Deadline是用户可见结果，仍抛`TimeoutError`；内部停止本身没有业务error或payload，用户不能
   `catch`。
3. Client或peer断开只表示response consumer已经消失。Runtime可以停止仍在运行的handler并丢弃晚到
   值、错误和Skiff heap写入，但不承诺撤销已经提交的外部副作用。
4. Internal transport中历史命名的`request.cancel`与`connection.request.cancel`可以暂时保留，
   但只能作为幂等、可丢失的stop hint。发送方必须先独立收束本地pending，业务正确性不能依赖peer收到
   hint。
5. 普通成功和业务错误路径必须严格等待transaction/lease的正常terminal。
6. 异常内部停止时，transaction在commit尝试开始前尽力abort；commit尝试开始后不得伪装为已经rollback，
   底层结果可以完成或保持unknown，晚到结果不能恢复已结束的request。
7. Lease异常停止时立即停止续租、尽力release；driver/session关闭和lease TTL是最终回收边界。可见request
   结果不等待cleanup acknowledgement，也不承诺cleanup恰好完成一次。
8. 第一版WebSocket JSON-RPC忽略所有notification，不接收或发送peer
   `$/cancelRequest`，也不定义`-32800 Request cancelled`。
9. Generic effect metadata继续描述effect kind、target、conflict key、idempotency和concurrency；
   第一版不要求每个host operation发布通用cancel-safety、commit point或cleanup action字段。

## 2. 更新的权威文档

- `doc/reference/runtime.md`
- `doc/reference/db.md`
- `doc/reference/service-yml.md`
- `doc/reference/std-surface.md`
- `doc/architecture/gateway-runtime-adapter-boundary.md`
- `doc/architecture/package-service-contract-deployment.md`
- `doc/architecture/open-issues.md`

其中`open-issues.md`原本仍把`$/cancelRequest`写成第一版唯一支持的notification；本节点把它改成
“第一版不支持任何notification handler或peer request cancellation”，消除了与其余权威设计的直接
矛盾。

## 3. 对旧任务的影响

`P5-F445H-O5D-db-terminal-lifecycle-owner-preflight-result.md`中的真实owner图、现有future drop风险、
lease renew task会detach等审计事实仍然有效；但其§4—§9要求的request级structured cleanup
supervisor、cleanup receipt/join、preserving-first-poll handoff和“异常路径也保证terminal恰好一次”
不再是当前实现合同，不得按该DAG继续开发。

`P5-F445H-O6-evaluator-db-state-machines.md`与其result中以下要求也已失效：

- 所有cancel/drop路径都必须等待或后台完成唯一abort/release；
- commit future真实`Pending`后必须由新owner继续驱动到provider terminal；
- 可见request结束后仍必须保留request级cleanup owner并等待receipt。

后继必须重发简化O6：普通路径仍使用E3的actual-Pending状态机且严格等待terminal；异常内部停止只保证
停止lease renew、隔离晚到结果和不无限持有request-local状态，DB资源使用best-effort cleanup及既有
driver/session/TTL回收。不得重新引入公开取消API或为此发明cleanup grace配置。

## 4. 边界

- Durable queue item自身的cancel/timeout状态机是持久任务语义，不等同于取消当前runtime request；
  本节点没有修改它。
- 代码中的`CancellationToken`、`Cancelled`、内部frame名和telemetry字段可以暂时保留为内部实现命名，
  但不能投影成业务API或WebSocket peer协议。
- 本节点没有执行WebSocket wire hard cut，也没有修改DB evaluator。权威设计完成不表示production已经
  符合；两项都必须由后继实现任务覆盖。

## 5. 验收

- `git diff --check`通过。
- 七份权威Markdown的fence数量均为偶数。
- `doc/reference`与`doc/architecture`中只剩以下与本决策相容的取消表述：
  - 明确列为unsupported的peer `$/cancelRequest`与`-32800`；
  - 历史内部frame/type名称及内部stop hint；
  - durable queue、后台work item等不同生命周期对象自己的取消语义。
- 未运行Cargo、stable instance、live、network或MongoDB。

