# P5-F360 typedJson unary correction

状态：Ready（C2 compiler语义纠正；可与F359 shared wire并行）。

## 直接父节点

- `P5-H36-external-ingress-implementation-dag.md`
- `P5-F357-http-gateway-compiler-projection-result.md`

以上父节点沿引用链连接唯一权威设计。2026-07-26权威文档已经进一步冻结：

- `../../../../architecture/package-service-contract-deployment.md`
- `../../../../architecture/gateway-runtime-adapter-boundary.md`
- `../../../../reference/runtime.md`
- `../../../../reference/static-semantics.md`

本任务只纠正F357误接受的`typedJson + Stream<T>`组合；不得重新设计HTTP authoring、raw HTTP stream、
gateway identity、RuntimeAssembly wire、Host/Router执行或WebSocket业务消息入口。

## Exact base

- integration commit：`344dae7fe29711709d3c435d6cc3e69726451456`
- integration tree：`07194037e113dc92c319a53b4ebb19cd33c62185`
- branch：`codex/package-service-phase-05`

该checkpoint已包含F357完整HTTP compiler projection、F358 linked gateway entry以及权威语义修订。
F359在独立worktree修改shared Rust/TypeScript request wire，与本任务compiler owner不重叠。

## 冻结规则

HTTP handler return矩阵只有三种合法结果：

```text
typedJson + unary external-schema-eligible T
rawHttp  + std.http.HttpResponse
rawHttp  + Stream<std.http.HttpResponseStreamEvent>
```

`typedJson + Stream<T>`对任意`T`都必须在compiler projection阶段结构化fail closed。`typedJson`不会把
每个stream item编码成JSON chunk，也不会生成server-stream `GatewayEntryProtocolSurface`。

## 必须完成

1. 修改HTTP gateway return projector：
   - 遇到`typedJson`的exact `Stream<T>` return直接返回entry-local handler validation error；
   - 错误必须明确说明`typedJson`只支持unary，HTTP streaming必须使用
     `rawHttp + Stream<std.http.HttpResponseStreamEvent>`；
   - 不得先为`T`做external schema projection后再失败，也不得生成`ServerStream` mode。
2. 保持raw HTTP规则不变：
   - unary仍要求exact compiler-owned `std.http.HttpResponse`；
   - stream仍要求exact compiler-owned `Stream<std.http.HttpResponseStreamEvent>`；
   - 错误owner、nullable、其它stream item继续fail closed。
3. 更新F357直接integration tests：
   - 删除`typedJson` server-stream正例及其gateway count/identity断言；
   - 增加至少一个schema-eligible item和一个不可投影item的`typedJson + Stream<T>`负例，证明拒绝由
     adapter kind/outer return决定，不依赖item schema；
   - 保留raw server-stream正例和raw item错配负例；
   - 证明所有成功的`typedJson` entry均为`Unary`且没有stream response schema。
4. 反向搜索compiler production/tests，确认没有把`typedJson`作为合法
   `GatewayDispatchMode::ServerStream`来源的剩余分支、fixture或断言。

## 写入范围

允许：

- `compiler/driver/http_gateway_projection/**`；
- `compiler/tests/http_gateway_projection.rs`；
- 若直接受影响，局部generated deployment compiler test。

禁止：

- artifact-model、identity canonicalization与deployment DTO；
- `service.yml` parser/authoring shape；
- RuntimeAssembly、runtime transport/loader/linker/Host/request/eval；
- Router、test-runner、std source；
- 三仓库service源码、stable/live配置、lockfile。

若实现需要改变shared DTO、identity preimage或runtime wire，立即返回`TASK_SCOPE_EXPANDED`，不得扩大任务。

## 验证

先枚举并确认非零selector，再运行：

```bash
cargo test -p skiff-compiler --test http_gateway_projection -- --list
cargo test -p skiff-compiler --test http_gateway_projection
cargo test -p skiff-compiler --test generated_service_deployment
cargo check -p skiff-compiler
rustfmt --edition 2021 --check compiler/driver/http_gateway_projection/mod.rs compiler/tests/http_gateway_projection.rs
git diff --check
```

反向搜索至少列出所有`GatewayAdapterKind::TypedJson`与`ServerStream`同函数命中并逐项分类。不得运行
workspace/root、stable/live，不push。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f360-typed-json-unary`
- branch：`codex/p5-f360-typed-json-unary`
- 从包含本task的integration checkpoint创建；
- production/tests一个commit，result一个commit；
- result写入同目录`P5-F360-typed-json-unary-correction-result.md`；
- result记录exact base/production commit/tree、负例、raw stream保留证据、nonzero selectors、验证命令与
  反向搜索；
- worktree保持clean，不merge/rebase integration，不push。
