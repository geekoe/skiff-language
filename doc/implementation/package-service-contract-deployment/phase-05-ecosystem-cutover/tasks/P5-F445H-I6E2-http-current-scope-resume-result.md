# P5-F445H-I6E2 HTTP current-scope consumer resume result

状态：

```text
TASK_SCOPE_EXPANDED
I6_HTTP_COMPLETE = NO
READY = NO
```

本任务在有界 production 调用链探查后停止。E1 已把
`OwnedExecutionControl` 交付到 Host HTTP adapter，但 adapter 的三个真实入口立即显式丢弃
carrier；该 adapter 文件不在 E2 唯一写集。没有实现 current-scope consumer，没有修改
production/tests，也没有新增 side channel、冻结值反推或其它影子实现。

## 1. 候选身份与实际写集

| 项 | commit / tree |
| --- | --- |
| E1 implementation commit | `ba66719e03cbabde2e159b94761cc1a1c71b35d2` |
| E1 implementation tree | `0b1972158d710c4355274f7fb272be292dcc7927` |
| integration base commit | `e942efa99460ea2b9bf29f07d8dfe855c9715aff` |
| integration base tree | `46abc10c8fbdab6e70f2ea071539382dbf03a1be` |
| task publication HEAD | `8628b37cd0056c550ef62ab40a5aa3e54b06baab` |
| task publication tree | `754b9ac86ead0a6012e06720024a4ee9ced5ece0` |
| implementation commit / tree | none |

实际写集只有本 result。production/test 写集为空。

## 2. 被证伪的合同前提

合同要求三个 HTTP open 操作开始时从 E1 carrier 读取完整 current scope，同时把 production
唯一写集限制为 concrete HTTP context、HTTP client runtime 与 transport。实际调用链为：

```text
capability-context HttpClientCapabilityContext
-> runtime/host/src/eval_capability_adapter/http.rs
   RuntimeHttpClientCapabilityContext::{dispatch_http_request,
     dispatch_http_stream, dispatch_http_sse}
-> runtime/host/src/capability_context/http.rs
   concrete HttpClientCapabilityContext
-> runtime/host/src/host/http_client_runtime.rs
```

精确 repo 证据：

1. `runtime/host/src/eval_capability_adapter/http.rs:18-56` 是 E1 carrier 与 concrete Host HTTP
   consumer 的唯一共同 owner。
2. 同文件第 24、36、53 行分别执行
   `let _execution_control = execution_control;`，随后调用不带 carrier 的 concrete dispatch。
3. `runtime/host/src/capability_context/http.rs:5-10` 的 concrete context 只保存 effects、
   HTTP options、stream runtime 与 test doubles；没有 execution carrier。
4. `runtime/host/src/host/http_client_runtime.rs:75-181` 的 unary/body-open/SSE-open 三个 concrete
   dispatch 均没有 carrier 参数，只能让 `HttpEffectRequest::new` 继续读取
   request-construction `deadline_ms` 与 root cancellation token。

因此 carrier 在进入 E2 授权 production 写集之前已经被丢弃。只修改授权文件无法满足“操作开始时
读取 full current scope”，也无法让 pending lower future观察 current scope的全部 signals与 absolute
deadline。

## 3. 为什么必须停止

安全实现至少需要修改
`runtime/host/src/eval_capability_adapter/http.rs`，让三个 adapter dispatch把收到的
`OwnedExecutionControl` 按值传给 concrete dispatch。该文件不在 E2 唯一写集。

若不修改该 owner，只能：

- 在 adapter 外新增全局/task-local mutable carrier；
- 从旧 `deadline_ms` 与 root token 反推 scope；
- 把 carrier冻结进 request-construction context。

这些方案都会违反 E1 唯一 carrier、operation-start current read与禁止影子实现约束。未发现需要修改
E4 stream owner、HTTP ingress、Router、公开 timeout/error surface、E1共享接口、Cargo manifest或
lockfile。

## 4. 最小后继合同

重新发行 E2 时，最小增量是把
`runtime/host/src/eval_capability_adapter/http.rs` 加入 production 写集，并严格只授权：

1. unary、body-open、SSE-open 三个 adapter method把已有
   `OwnedExecutionControl` 传给 concrete dispatch；
2. concrete dispatch / `HttpEffectRequest::new` 在 operation start调用一次
   `execution_scope()`；
3. 原合同授权的 shared scoped lower-future owner继续负责 lease、winner、drop、late completion
   fence与 cleanup；
4. 不改变 public capability trait、业务参数、E4 handle-established stream owner或任何 wire
   surface。

这只是一个精确漏列的 Host adapter bridge owner，不需要修改 Eval或 E1 shared carrier接口。

## 5. RED / GREEN、矩阵与验证

由于 current carrier无法在授权范围内到达 concrete HTTP consumer，合同要求的真实 RED 无法安全
建立，未新增零价值的内部 helper测试，也未运行合同 Cargo命令。

| 项 | 数量 / 结果 |
| --- | --- |
| RED listing / execution | `0 / 0`（未建立） |
| GREEN listing / execution | `0 / 0`（未建立） |
| unary | BLOCKED before authorized owner |
| body-stream open | BLOCKED before authorized owner |
| SSE open | BLOCKED before authorized owner |
| lease/timer/waiter cleanup evidence | 未建立 |
| late lower completion fence evidence | 未建立 |

只读探查使用 `git`、`rg` 与源码阅读。没有运行 full gate，没有访问或启动
stable/live/network/MongoDB，没有 merge、rebase或 push。result提交前只运行
`git diff --cached --check`。

```text
I6_HTTP_COMPLETE = NO
```
