# P5-F427 HTTP correlation field removal batch

状态：Ready for bounded audit。2026-07-27用户修正了HTTP payload语义。

## 直接父节点与权威设计

- `P5-F426-connect-wire-and-http-consumer-wave.md`
- `P5-F425B-aihub-http-stream-service-result.md`
- `P5-F425C-aihub-http-stream-client-result.md`
- `P5-F425D-agine-user-http-service-checkpoint-result.md`
- `doc/architecture/gateway-runtime-adapter-boundary.md`
- 最终事实源：`doc/architecture/package-service-contract-deployment.md`

权威设计在Skiff `42337081095cce9c618508b9938cf28516054a75`冻结：

- HTTP request天然与自己的unary response/server stream关联；
- external HTTP payload、response envelope和stream item不声明只用于模拟旧WebSocket req/res的
  `requestId`、`correlationId`或同义字段；
- platform request/trace id不进入业务schema；
- 真正业务语义分别使用`idempotencyKey`、`jobId`、`runId`或资源ID。

## 精确代码状态

| Repo | Commit | Tree |
| --- | --- | --- |
| Skiff integration | `42337081095cce9c618508b9938cf28516054a75` | `da930e4ba3674c4913690664a537af4c5cfe0b23` |
| Internals integration | `ed5d333b2406d5375fca8acc96f4695667c48ced` | `26024bd221af3bb745c40039c8bf70e59ef1fc23` |

F426A connect wire不受影响并继续执行。F426B/C旧合同已supersede；F426C未完成实现封存为Internals
WIP `62c3d6342ab81210e15c5ebb9e56cb17ae66a9f6` / tree
`45e0c12486fd523773a6a0eaff1293044cb67182`，只可作为差分输入，不属于integration candidate。

## 范围与DAG

先并行只读：

```text
F427A  Agine HTTP requestId/correlation owner audit
F427B  AIHub HTTP event requestId/correlation owner audit
```

审计必须区分：

- HTTP external correlation-only字段：删除；
- legacy WebSocket req/res字段：在旧receive最终cleanup前可暂留，不能反向污染HTTP；
- service-call/provider/internal protocol字段：只有同样被证明是HTTP残留时才删除；
- `runId`、`jobId`、resource ID或幂等键：按真实业务语义保留，不因名称相邻误删。

结果合流后分别签发Agine与AIHub repair leaf。Repair完成前，F425B/C/D中涉及HTTP wire identity、
generated receipt或combined的证据失效；其它独立业务实现可作为代码检查点保留。

本batch不访问stable/live/instance，不运行完整N5，不merge/rebase/push。
