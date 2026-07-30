# P5-F445H I7 P8 S1 PackageDirect HTTP stream registry closure result

状态：

```text
TASK_NOT_EXECUTABLE
S1_COMPLETE = NO
I_RESUME_UNBLOCKED = NO
SCOPE_EXPANDED = NO
```

## 1. Baseline and completed checkpoint

- baseline：
  `2bcb40e61ee6b922eeca913651e2cc344a38b50e`
  （tree `df2bd49666a55d73f69b63b38c267bda8d2aed9d`）
- task branch：
  `codex/p5-f445h-i7-p8-s1-stream-registry`
- worktree：
  `/Users/geek/workspace/skiff-p5-f445h-i7-p8-s1-stream-registry`
- 有效诊断fixture提交：
  `77f1c5b26f0b00b1e47acb0716fa43e1eb2f9c36`
  （tree `8df4e269c4e62f06c72c4660a4b56433447b4121`）

该提交只增加真实交叉fixture和行为测试，没有production修改。fixture经过：

```text
compiled package/service authoring
  -> linked/admitted RuntimeAssembly
  -> concrete Host request entry
  -> rawHttp serverStream gateway
  -> consumer package wrapper
  -> helper dependency PackageDirect stream producer
  -> real Host response sink
  -> response.start / response.chunk / response.end
```

## 2. Required RED was disproved

在未修改production语义的candidate上增加临时task-local trace，记录concrete
`StreamRuntime`的registry指针、request generation和stream id。正确完成fixture authoring后，同一测试连续
三次GREEN；其中两次完整重复trace为：

```text
open   registry=A generation=1 stream=-
create registry=A generation=1 stream=stream-0
lookup registry=A generation=1 stream=stream-0
create registry=A generation=1 stream=stream-1
lookup registry=A generation=1 stream=stream-1
... stream-0 / stream-1 lookup均为registry=A、generation=1 ...
close  registry=A generation=1 stream=-
```

独立进程中的指针值不同，但单次请求内所有create/lookup/close恒为同一registry。`stream-0`是HTTP wrapper
producer，`stream-1`是依赖package的`PackageDirect` producer。两者都属于request generation `1`。

因此：

- create与lookup使用相同registry identity；
- create与lookup使用相同request generation；
- create与lookup使用相同stream id；
- response按一个start、一个chunk、一个end结束，且没有第二个terminal；
- request scope在两个stream完成后关闭；
- 没有出现`unknown Stream value`或其它行为RED。

临时trace已全部撤回；最终diff中不存在环境变量、日志、public API或instrumentation。

## 3. Owner lifetime and first divergence

当前真实链路中：

1. `ProgramExecutionContext::with_runtime_assembly_target`打开request generation scope并持有
   `StreamRuntimeOwner`；
2. `Interpreter::execute_runtime_http_gateway_server_stream`在调用
   `execute_runtime_assembly_addr_with_stream_defer(context.clone(), ...)`时保留原始`context`；
3. wrapper deferred producer与其`PackageDirect` producer都从该context捕获同一个scoped
   `StreamRuntime`；
4. gateway消费完成后原始context释放owner并关闭scope。

首个偏离既定语义的production symbol：**未找到**。任务合同要求“无稳定RED时不得修改production”，并明确
将“稳定RED无法建立”列为强制停止条件；继续修改任何association/lifetime owner都会成为猜测性修复。

## 4. Evidence

执行：

```text
SKIFF_TRACE_PACKAGE_DIRECT_STREAM_REGISTRY=1 \
  cargo test --locked -p skiff-runtime-host \
  package_direct_http_stream_registry_return_stream_reaches_real_gateway -- --nocapture
```

正确fixture连续三次均为：

```text
1 passed; 0 failed
```

撤回trace后的最终checkpoint执行：

```text
cargo test --locked -p skiff-runtime-host \
  package_direct_http_stream_registry_return_stream_reaches_real_gateway -- --nocapture
cargo fmt --all -- --check
git diff --check
```

全部通过。未运行任务其余completion matrix、full workspace、生态gate、stable/live/network/Mongo/OAuth/browser；
原因是强制停止发生在required RED gate，不能把已有GREEN扩张成无因果production实现或伪造`S1_COMPLETE`。

## 5. Actual write set and next step

实际写集：

```text
runtime/host/src/host/router_session/tests/runtime_assembly_request.rs
runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs
test-runner/fixtures/package-direct-http-stream-registry/**
本result
```

建议保留并集成诊断fixture，因为它关闭了“普通wrapper→PackageDirect return stream的registry association已经
失败”这一假设；但该提交不能解除I。

后续只读差分已经把第一个结构差异缩小到“overlay-local stream producer返回值作为另一个dependency
PackageDirect stream producer的参数”。该差分由
`P5-F445H-I7-P8-D3-stream-argument-response-sink-refinement-result.md`固化，并交给S2/S3顺序实验。
本S1状态保持不变：

```text
TASK_NOT_EXECUTABLE
S1_COMPLETE = NO
I_RESUME_UNBLOCKED = NO
```

不能继续以泛化的`PackageDirect HTTP stream registry`为production修复任务，也不能从现有GREEN轨迹或
只读差分推断第二个registry、owner过早drop、heap、overlay association、argument transport或response
sink已经是根因。
