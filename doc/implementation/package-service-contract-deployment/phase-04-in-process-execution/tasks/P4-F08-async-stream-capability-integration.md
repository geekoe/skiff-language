# P4-F08：Async Error / Stream Capability Integration

## 权威输入、风险与证据状态

- 执行输入：R02在`ee1609c`的blocking issues 2、3及T06 host合流回归；F06提供lane-neutral error planner，
  F07提供canonical callback projection。
- 风险/验收组：高风险跨lane async/stream integration；由R02复验。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：F06、F07 exact commits均已合流integration。
- 解锁：R02 retry。
- branch：`codex/p4-f08-async-stream-capability-integration`。
- worktree：`/Users/geek/workspace/skiff-p4-f08-stream-capability`。
- 五分钟内真实edit；T05 stream owner执行，消费F06/F07公共API，不改mapping/materializer语义owner。

## 写入范围与完成态

- 独占async/stream lane、provider emit到internal stream carrier seam、concrete stream runtime/capability lifetime与owned
  async host测试；为闭合emit可做`runtime/eval/src/eval_context.rs`的最小delegate。不修改ordinary/callback mapping/compiler/router。
- async unary success与declared/undeclared typed error全部调用F06 planner；schema/plan/detachment/error分类与sync一致，
  cancel仍优先按descriptor语义处理。
- callback-capability stream item必须在ordinary JSON encoding前调用F07 projection；内部in-process stream carrier可保存
  detached runtime value/opaque capability，consumer不得再次JSON round-trip。普通canonical items仍保持既有typed编码。
- stream lease必须在projection前active；producer emit、consumer next、terminal publish/backpressure、early break、close、
  cancel与owner exit均exact-once释放capability/registry/task，过期lookup稳定返回设计错误。
- host/eval正负例必须覆盖真实local interface item→opaque capability→consumer invocation、wrong mapping/tuple、close/cancel
  expiration，以及async typed error detached。修复callback host合流断言并清理execution root既有unused import。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-eval service_error_boundary
cargo test -p skiff-runtime-eval in_process_stream
cargo test -p skiff-runtime-eval in_process_callback
cargo test -p skiff-runtime-host stream_runtime
cargo test -p skiff-runtime-host typed_execution_async_stream_cancel
cargo test -p skiff-runtime-host typed_execution_callback_native
git diff --check
```

所有过滤器必须非零PASS；不得运行完整runtime gate。

## 回报

提交一个clean commit，回报async error交接、stream carrier状态图、projection/lifetime时序、host动态证据与残余风险。
