# P5-F445H I7 P8 S3 Deferred response sink propagation

状态：

```text
READY_AFTER_S2_PASS
BLOCKED_BY = S2_PASS
I_RESUME_UNBLOCKED = NO
```

## 1. Parent and baseline

- 直接父节点：
  `P5-F445H-I7-P8-S2-stream-producing-argument-transport.md`及其PASS result。
- dispatch时必须提供S2已集成后的精确Skiff commit/tree；S2未最终GREEN不得启动。
- evidence-rule correction baseline：
  `405cc99b3af136429a52ecf8b85dbf7d044e5438`
  （tree `30a74f00a971055abcbbc59b2f37f7fcca80ad17`）。
- S3 RED fixture checkpoint：
  `e45e81e8f37c1734fd25e16d72ddbf7ff68c757e`
  （tree `efd1872329a86bae50b55289ead23d12cc296018`）；该checkpoint尚未表示S3完成。
- repo：Skiff。
- integration owner：`/root/phase05_integration_steward`。
- DAG：`S2 -> S3 -> I resume -> X`。

S3沿用S2的compiled/linked/admitted `kind:test` fixture、`RuntimeHost` router-session binary frame
dispatch和concrete Host response sink，只验证Router以下的deferred response-sink env handoff。standalone
Router ordinary ingress继续组合T的既有证据；本任务不启动Router business port，也不能单独声称请求经过
standalone Router。若S3 candidate修改T覆盖的Router/Runtime ingress owner，必须先重验T，不能沿用失效
checkpoint。

## 2. One independent delta

从S2最终GREEN fixture建立新实验。保持overlay-local `source() -> Stream<string>`作为dependency
producer参数；只把dependency `wrap`改成在消费参数后调用现有
`std.http.emitResponseStream`：

```skiff
function wrap(
  input: Stream<string>
) -> Stream<std.http.HttpResponseStreamEvent> {
  emit(std.http.streamStart(200, Array.empty<std.http.HttpHeader>()))
  for value in input {
    std.http.emitResponseStream(
      std.http.streamChunk(bytes.fromUtf8(value))
    )
  }
  emit(std.http.streamEnd())
  return null
}
```

该函数仍是deferred PackageDirect stream producer；start/end走其语言stream，chunk专门检查当前raw HTTP
request已有response sink是否传播到dependency deferred producer env。不得同时改变argument transport、
source owner、Host router-session lower seam、manifest identity、类型或取消时机。

预期Host response frames仍严格为一个start、`"body"` chunk和一个end。若平台现有single-terminal规则要求
native chunk与outer stream串行化，fixture按`RouterWriterMessage`实际frame顺序断言，不按network
socket或chunk边界断言。

## 3. Trace and verdict

在S2 trace字段基础上，另记录每一层executable进入/退出时：

```text
response sink present/absent
response sink identity
stream sink identity
request generation
native emitResponseStream target/result
```

临时trace不能输出payload或新增公共日志协议，最终必须撤回。

- 起点GREEN：保持production NO-OP，补齐正常/error/cancel证据后解除I；
- 稳定得到`emitResponseStream ... outside ... context`或其它response-sink RED：定位sink最后存在与首次
  缺失的相邻existing env handoff；
- `unknown Stream value`只有在stream create/register/lookup的stream id、registry identity或request
  generation首先出现缺失/不一致时，才退回S2分类；
- 若三个stream的create/lookup identity完整一致，而首个偏离是deferred producer env缺少existing
  response sink，则仍归S3；native失败后的cancel/cleanup最终暴露`unknown Stream value`只记为secondary
  cleanup error，不能覆盖更早的response-sink根因；
- 若trace无法确定response-sink缺失与stream identity偏离的先后，停止并补诊断，不按最终错误文本猜测；
- 若错误不属于argument transport或response sink，停止上报。

当前冻结RED已经连续两次证明：

```text
stream-0 / stream-1 / stream-2
  -> same registry
  -> request generation = 1
  -> create/register/lookup identity consistent

runtime_http_gateway serverStream
  -> execute_runtime_assembly_addr_with_stream_defer(..., Env::new())
  -> outer deferred stream-0 created with response sink absent

native emitResponseStream enter
  -> response sink absent
  -> current stream sink present
  -> native response-context failure
  -> cancel/cleanup masks terminal as unknown Stream value
```

`program_invocation`中设置response sink的路径没有经过，不能把它的既有GREEN当作本路径证据。上述内容只是
允许继续S3的RED分类，不是implementation或GREEN。

正常/error/cancel可在同一Host lower seam验证，其中cancel只证明runtime `request.cancel` frame。若目标或
失败分类要求external socket/client disconnect、Router framing或business-port backpressure，必须停止并
拆出standalone Router实验；不得把Host frame cancellation冒充外部断连证据。

## 4. Bounded implementation

只有两次相同RED、sink轨迹存在唯一首次丢失点时，才允许修复existing response sink env propagation。
production候选owner限于：

```text
runtime/eval/src/program_stream.rs
runtime/eval/src/runtime_http_gateway.rs
```

冻结的最小repair是：

1. outer deferred producer创建后仍以既有stream id登记到现有deferred producer registry；
2. `runtime_http_gateway`从返回的stream value取得该exact id，在drive前把outer producer已经拥有的
   stream sink以既有`TypedStreamSink` view附着到同一个parked producer env；
3. `program_stream`只提供完成上述exact-id内部附着所需的最小crate-private能力，并沿原
   `drive_deferred_stream_producer`路径启动；
4. missing id、已被take、registry/request不一致或重复附着必须fail closed。

只能复用outer deferred producer已经创建的existing stream sink、item type plan及既有
`TypedStreamSink`表达；禁止创建第二个channel/sink owner、第二registry、全局状态、公共API、特殊header
或测试专用context。不得修改`program_execution`、`program_invocation`、`env`或其它owner来绕过冻结路径。
修复不能让非raw HTTP调用获得response sink，也不能把sink传播到另一个request/service boundary。

最终必须证明：

- 正常响应一个start/chunk/end；
- producer error与native sink error只有一个terminal，三个stream全部finish；
- consumer break/cancel停止dependency producer和overlay source，无晚到chunk；
- raw HTTP context外直接调用仍以既有错误fail closed；
- S2 argument transport与S1普通return stream保持GREEN；
- request close时stream与response sink owner均归零。

GREEN还必须证明原RED中的native response-context failure消失，并且Host response terminal不再被cleanup
`unknown Stream value`遮蔽；不能只改错误优先级或吞掉cleanup错误。

## 5. Write set and evidence

测试只扩展S2同一真实fixture owner：

```text
runtime/host/src/host/router_session/tests/runtime_assembly_request.rs
runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs
test-runner/fixtures/package-direct-http-stream-registry/**
```

result记录实际selector和命令：

```text
cargo test --locked -p skiff-runtime-host \
  deferred_package_direct_stream_keeps_raw_http_response_sink -- --nocapture
cargo test --locked -p skiff-runtime-host \
  package_direct_stream_producer_argument_real_gateway -- --nocapture
cargo test --locked -p skiff-runtime-host \
  package_direct_http_stream_registry_return_stream_reaches_real_gateway -- --nocapture
cargo test --locked -p skiff-runtime-eval emit_response_stream -- --nocapture
cargo check --locked -p skiff-runtime-eval -p skiff-runtime-host
cargo fmt --all -- --check
git diff --check
```

selector中的`real_gateway`仍是历史名称，不是standalone Router证据。fixture不得直接调用handler、手工
构造Interpreter或使用mock response sink。

不运行完整AIHub或J gate。S3通过后由I owner在冻结candidate上恢复四条AIHub迁移。

result必须区分并逐项报告：

- RED中三个stream的create/register/lookup identity与首个偏离；
- native response sink/current stream sink的presence和identity；
- 最终`unknown Stream value`是primary stream偏离还是secondary cleanup；
- 实际production写集是否严格为上述两个owner；
- existing sink exact-id附着发生在deferred producer drive之前；
- 是否新增sink/channel/registry/state/public API，预期全部为NO；
- normal/error/cancel、outside-context负例与S1/S2回归。

## 6. Prohibitions and stop conditions

禁止new registry/protocol/schema/compiler/Router/test-runner/std/Internals production，禁止在同一提交重新
设计stream argument transport。若无稳定RED、sink从未丢失、需要公共surface、跨request/service sink、
需要修改冻结的两个production owner之外的文件、需要external disconnect/Router framing、多个owner同时
改动或有多个实现方向，返回
`TASK_NOT_EXECUTABLE`/`TASK_SCOPE_EXPANDED`，不做猜测性修复。

## 7. Handoff

提交fixture、最小实现（如有）与result，报告精确commit/tree、实际写集、sink/stream身份轨迹、
RED/GREEN/error/cancel矩阵和外部context负例。只有全部闭合才能设置：

```text
S3_COMPLETE = YES
I_RESUME_UNBLOCKED = YES
```

交给`/root/phase05_integration_steward`集成与清理；不得自行写integration、merge、push或恢复I。
