# P5-F445H I7 P8 S2 Stream-producing argument transport

状态：

```text
READY_FOR_ZERO_WORKTREE_PREFLIGHT
BLOCKED_BY = D3_INTEGRATION
I_RESUME_UNBLOCKED = NO
```

## 1. Parent, baseline and ownership

- 直接父节点：
  `P5-F445H-I7-P8-D3-stream-argument-response-sink-refinement-result.md`
- ancestry floor：
  `44e83695d5d9e6559b3ac5f482b9faffd1f96cb3`
  （tree `6cc2284797d52a6d3549afb255eeaae6247a6915`）。
- dispatch时必须提供D3已集成后的精确Skiff commit/tree。
- repo：Skiff。
- integration owner：`/root/phase05_integration_steward`。
- DAG：`T -> S1 diagnostic -> S2 -> S3 -> I resume`。

S2不能声称完成S1，也不能直接恢复I。只有S3在S2最终GREEN后可以解除I。

## 2. Required real fixture

复用S1的compiled/admitted/real Router与Host链，但在同一fixture tree新增独立`kind: test` service，
不改变S1现有consumer。该test service的`http.yml`显式引用自己的`*.test.skiff` overlay entry，并以普通
public alias `helper`直接依赖provider；provider `api.yml`只公开`wrap`。主实验的源码形状固定为：

```skiff
// kind:test overlay source
function source() -> Stream<string> {
  emit("body")
  return null
}

function entry(request: std.http.HttpRequest) -> Stream<std.http.HttpResponseStreamEvent> {
  for event in helper/wrap(source()) {
    emit(event)
  }
  return null
}
```

```skiff
// dependency production source
function wrap(
  input: Stream<string>
) -> Stream<std.http.HttpResponseStreamEvent> {
  emit(std.http.streamStart(200, Array.empty<std.http.HttpHeader>()))
  for value in input {
    emit(std.http.streamChunk(bytes.fromUtf8(value)))
  }
  emit(std.http.streamEnd())
  return null
}
```

必须通过真实Router business ingress进入linked/admitted rawHttp `serverStream` entry，并由真实Host
response sink看到一个start、`"body"` chunk和一个end。禁止直接调用handler、手工构造Interpreter、
mock response sink或绕过test service overlay。

## 3. Required trace and primary verdict

临时task-local trace必须为三个stream分别记录：

```text
stream role
stream id
registry identity
request generation
create/register
lookup/first next及后续next
cancel
finish/terminal
owning executable address + package build/module/symbol
```

三个role是HTTP entry producer、dependency `wrap` producer和overlay-local `source` argument producer。
还要记录request scope open/close、活动stream归零、single-terminal和首个失败事件。指针、临时路径和
payload必须去敏；最终production与fixture不能保留环境变量或日志协议。

主实验包含normal、producer error和consumer break/cancel三个case；每个case都必须覆盖三个stream，
其中normal负责start/chunk/end，另外两个负责cancel/finish轨迹。只有normal在未修改production的candidate
上连续两次于同一个首次next得到`unknown Stream value`，才能进入下一节的RED对照。其它失败直接停止
分类。三个case全部GREEN才可把S2判为起点GREEN。只出现旧错误文本或AIHub历史日志不能替代本fixture。

## 4. RED-only controlled comparison

只有normal主实验稳定得到上述首次next RED时，才在下一次独立运行中把`source()`机械移入dependency，
并让dependency `wrapLocal()`以相同值、相同三层producer和相同HTTP entry输出消费它。该对照可在
provider `api.yml`临时公开`wrapLocal`，最终是否保留由result记录；其它manifest、类型、event、
Router/Host链、取消时机与trace均保持不变。

判定：

| 主实验 | dependency-local对照 | 分类 |
| --- | --- | --- |
| RED | GREEN | overlay→dependency argument association/transport候选 |
| RED | 同一首次next RED | general stream-producing argument transport候选 |
| RED | 不同失败或非确定 | 未隔离；停止，不修复 |

不能同时保留两套猜测或在得到对照前修改production。

## 5. Bounded implementation

仅在稳定RED、对照得到单一分类且首个偏离symbol唯一时，允许修复现有stream-producing argument
prepare/materialize/drive链。production候选owner限于：

```text
runtime/eval/src/program_stream.rs
runtime/eval/src/eval_context.rs
runtime/eval/src/program_execution.rs
```

实际只改直接拥有偏离的最小文件。必须复用当前request的`StreamRuntime`、现有
`StreamInternalItem`/wire-to-heap搬运、现有cancel signal和现有deferred producer registry；不能创建
第二个registry或以字符串重新查找stream。

修复后主实验和dependency-local对照都必须GREEN，并覆盖：

- 正常start/chunk/end与active=0；
- producer error保留已发item、单terminal并清理三个stream；
- consumer break/cancel传播到`wrap`和`source`，无晚到response写入；
- nested argument producer只消费一次；
- S1原普通PackageDirect return stream保持GREEN。

主实验三个case起点即GREEN时，S2保持production NO-OP，不运行RED-only对照，撤回trace并交S3。

## 6. Write set and evidence

预期test owner：

```text
runtime/host/src/host/router_session/tests/runtime_assembly_request.rs
runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs
test-runner/fixtures/package-direct-http-stream-registry/**
```

允许在同一fixture树增加overlay/dependency source和manifest；不修改test-runner production。
selector可机械调整，result必须记录实际命令：

```text
cargo test --locked -p skiff-runtime-host \
  package_direct_stream_producer_argument_real_gateway -- --nocapture
cargo test --locked -p skiff-runtime-host \
  package_direct_http_stream_registry_return_stream_reaches_real_gateway -- --nocapture
cargo test --locked -p skiff-runtime-eval stream_producer_arg -- --nocapture
cargo check --locked -p skiff-runtime-eval -p skiff-runtime-host
cargo fmt --all -- --check
git diff --check
```

不运行I、完整AIHub、J生态gate、stable/live/network/Mongo/OAuth/browser。

## 7. Prohibitions and stop conditions

禁止：

- new registry、跨request共享registry、测试专用bridge；
- 新协议、header、schema、artifact代际、compiler、Router、test-runner或std production修改；
- service boundary改写或Internals修改；
- 在S2中顺手加入`std.http.emitResponseStream` response-sink实验；
- 无稳定RED或无单一对照结论时修改production。

若主实验/对照无法稳定复现、trace缺任一stream/executable、失败属于response sink、修复需要超出候选
owner、或仍有多个会改变实现方向的未知量，返回`TASK_NOT_EXECUTABLE`或`TASK_SCOPE_EXPANDED`。保留
最小诊断fixture/result，不猜测性修复。

## 8. Handoff

提交fixture、最小实现（如有）与result，报告commit/tree、实际写集、两次主轨迹、RED-only对照（若
运行）、RED/GREEN矩阵、三个stream的cancel/finish与`S2_COMPLETE`。S2最终GREEN时只设置
`S3_UNBLOCKED=YES`，保持`I_RESUME_UNBLOCKED=NO`。

交给`/root/phase05_integration_steward`集成与清理；不得自行写integration、merge、push、启动S3或恢复I。
