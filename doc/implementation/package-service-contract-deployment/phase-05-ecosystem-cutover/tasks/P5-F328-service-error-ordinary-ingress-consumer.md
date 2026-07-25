# P5-F328 Service error ordinary and ingress consumer

状态：Ready。

## 直接父节点

- R0 frozen API acceptance：
  `P5-F327-service-error-core-independent-acceptance-result.md`
- current real-path/consumer audit：
  `P5-F319-service-error-channel-delta-audit-result.md`

本任务实现F319的R1。R0 API已冻结；不得改变envelope、imported cause、selected codec、type index、platform
registry或export/import签名。

## DAG与并行边界

- 与F329 async/stream/cancel及F330 service test effect并行。
- 本任务唯一拥有central in-process dispatcher的fixed-error import/origin分流；F329只让async lane产出同一
  fixed carrier，F330不修改dispatcher。
- 完成后解除ordinary/ingress A5子门及W2-W ingress handoff；不代表R2/R3或A5完成。
- 证据基线：worktree创建HEAD。R0 API、ordinary provider heap lifecycle、dispatcher origin或ingress
  adaptation变化会使证据失效。

## Production写入范围

- `runtime/eval/src/assembly_execution/mod.rs`仅fixed failure的central origin分流/import
- `runtime/eval/src/assembly_execution/boundary_materialization.rs`
- `runtime/eval/src/assembly_execution/ordinary.rs`
- `runtime/eval/src/assembly_execution/ingress.rs`
- `runtime/eval/src/assembly_execution/websocket_ingress.rs`

测试只限上述模块co-located tests、`ordinary/tests.rs`与`ordinary/test_runtime.rs`。禁止修改R0 core、
async/stream/cancel、test effect、capability/request/host/transport/router/std/compiler/artifact及权威设计。

`mod.rs`是对F319原R1范围的明确可执行性修正：internal与ingress共享dispatcher，只有这里持有
`InProcessBoundaryDispatchOrigin`，因此origin-specific import必须由R1唯一修改；不能让ordinary lane猜origin。

## 完成标准

### Provider export

- ordinary provider使用fresh heap和provider-local stack scope执行；
- provider error在heap drop前调用R0 `export_provider_failure`，不再原样返回provider
  `UserException`或`materialize_provider_error` passthrough；
- imported fixed cause、public/dependency error、private/Internal/platform均只走同一core；
- success directional materialization、same-heap package direct与callback行为不变。

### Dispatcher import/origin

- `InternalServiceCall`收到`FixedServiceFailure`时，在caller heap/call site/current local stack调用R0 import，
  返回caller-local`UserException`；
- `Ingress`/WebSocket ingress只把fixed carrier向上交给W2-W，不创建虚构external caller exception；
- generic non-fixed provider boundary error不能绕过分类；caller cancellation/control error不能冒充provider
  response；
- service/operation/errorId RemoteBoundary字段来自resolved target和fixed envelope，不从message猜。

### 逐跳与heap

- provider heap销毁后caller value仍只引用caller heap；
- exact linked public/Internal可catch；unlinked public catch miss但再次service export原bytes；
- A→B→C public与Internal路径：每跳相同payload/traceId/errorId，B/C各有自己的local stack；
- ingress fixed bytes不含callee source/path/function/private payload。

## 探针

至少覆盖ordinary/ingress真实B1、B2、B3、B4、B7、B8、B8a、B9及S1/S2：

- public record/representation或union至少一个真实execution image；
- dependency owner、private→Internal、platform与Resource分流；
- wrong owner/key/id、provider heap drop、ingress不import；
- local rethrow对照remote import新stack。

```bash
cargo test -p skiff-runtime-eval --lib assembly_execution::ordinary -- --list
cargo test -p skiff-runtime-eval --lib assembly_execution::ordinary --no-fail-fast
cargo test -p skiff-runtime-eval --lib assembly_execution::ingress --no-fail-fast
cargo test -p skiff-runtime-eval --lib assembly_execution::websocket_ingress --no-fail-fast
cargo check -p skiff-runtime-eval --lib
git diff --check
```

selector必须非零。generic WebSocket source compiler的两个既知失败不属于本节点；不运行完整eval/workspace/
stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f328-error-ordinary`
- branch：`codex/p5-f328-error-ordinary`
- 风险：高；新的一次性Agent，5分钟内先替换ordinary passthrough并接central origin分流；
- 提交并返回provider/export、dispatcher/import、ingress、三跳/heap矩阵；
- 不push、不承接R4或验收。

