# P5-F329 Service error async, stream and cancel consumer

状态：Ready。

## 直接父节点

- R0 frozen API acceptance：
  `P5-F327-service-error-core-independent-acceptance-result.md`
- current real-path/consumer audit：
  `P5-F319-service-error-channel-delta-audit-result.md`

本任务实现F319的R2。R0 API已冻结；不得修改core或ordinary central dispatcher。

## DAG与并行边界

- 与F328 ordinary/ingress及F330 service test effect并行。
- 本任务拥有async unary、server stream terminal、cancellation分流、typed capability response/stream carrier和
  legacy outbound seam。
- 完成后解除async/stream/cancel A5子门及W2-W typed response handoff。
- 证据基线：worktree创建HEAD。R0 API、stream terminal/lifetime、capability response enum或cancellation
  ordering变化会使证据失效。

## Production写入范围

- `runtime/eval/src/assembly_execution/async_stream_cancel.rs`
- `runtime/eval/src/program_stream.rs`
- `runtime/capability-context/src/stream.rs`
- `runtime/capability-context/src/response.rs`
- `runtime/capability-context/src/outbound_response.rs`
- `runtime/eval/src/service_dispatch.rs`

测试只限上述文件co-located modules及
`runtime/capability-context/src/lib.rs`中service-response/stream focused tests。禁止修改
`assembly_execution/mod.rs`、ordinary/ingress、test effect、R0 core、request/host/transport/router/std/
compiler/artifact及权威设计。

## 完成标准

### Async unary

- provider heap存活时用R0 export，产出与ordinary相同的`FixedServiceFailure`；
- F328 central dispatcher合流后负责internal import；本lane不复制classifier/import；
- caller/request cancellation select继续是control path，不生成provider Internal/Platform envelope。

### Server stream

- provider task在heap存活时把terminal error导出为typed fixed service error；
- stream carrier明确区分fixed service failure与local/general dynamic producer error；
- fixed service failure跨task/heap仍保留原bytes，不依赖`Box<dyn WirePayload>` downcast/code；
- consumer使用R0 import；opaque hop不decode/re-encode；
- consumer cancellation、request cancellation和normal end保持现有lifecycle，不被分类为provider response。

### Capability/legacy seam

- capability response/outbound enum拥有明确fixed service-error variant或等价typed carrier；
- `service_dispatch::outbound_router_response_into_result`只透传typed fixed error；
- generic `ResponseError`不再按`message`统一变成ProviderUnavailable，未迁移generic producer必须Protocol/
  fail closed；
- 不把control-plane generic error全局改成业务error，不在capability/eval复制public/private/platform规则。

## 探针

至少覆盖async/stream B1、B3、B6、B8、B9、S1/S2以及：

- provider heap销毁后linked import成功；
- unlinked opaque stream raw bytes不变；
- platform typed、Resource不platform；
- generic legacy error不能按message分类；
- caller/consumer/request cancellation不生成Internal；
- terminal ordering、normal close、cancel cleanup不回归。

```bash
cargo test -p skiff-runtime-capability-context --lib -- --list
cargo test -p skiff-runtime-capability-context --lib --no-fail-fast
cargo test -p skiff-runtime-eval --lib assembly_execution::async_stream_cancel -- --list
cargo test -p skiff-runtime-eval --lib assembly_execution::async_stream_cancel --no-fail-fast
cargo test -p skiff-runtime-eval --lib program_stream --no-fail-fast
cargo test -p skiff-runtime-eval --lib service_dispatch --no-fail-fast
cargo check -p skiff-runtime-eval --lib
git diff --check
```

selector必须非零。不得运行完整eval/workspace/root/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f329-error-stream`
- branch：`codex/p5-f329-error-stream`
- 风险：最高，stream/lifetime/cancel；新的一次性Agent，5分钟内先建立typed carrier并替换provider terminal；
- 提交并返回async/stream/cancel/capability/legacy矩阵和lifecycle证据；
- 不push、不承接R4/W2-W或验收。

