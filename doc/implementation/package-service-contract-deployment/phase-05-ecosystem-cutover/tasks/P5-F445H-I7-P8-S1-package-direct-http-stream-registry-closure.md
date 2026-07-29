# P5-F445H I7 P8 S1 PackageDirect HTTP stream registry closure

状态：

```text
READY_FOR_ZERO_WORKTREE_PREFLIGHT
BLOCKED_BY = D1_INTEGRATION
```

## 1. Parent, baseline and DAG

- 直接父节点：
  `P5-F445H-I7-P8-D1-package-direct-http-stream-task-refinement-result.md`
- 架构事实源：
  `../../../../architecture/package-service-contract-deployment.md`
- ancestry floor：
  `ff6418f5a43ee503608cf8f54512bd9f53a47a74`
  （tree `a6ea21c20231e40db69960f70cc6850a7723f871`）
- dispatch时必须由主Agent提供D1已集成后的精确Skiff commit/tree；零worktree预检锚定该commit，不能
  直接读取移动中的integration工作区。
- DAG：`T -> S1 -> I resume -> X`
- repo：Skiff
- integration owner：`/root/phase05_integration_steward`
- 默认使用新的有界开发Agent；完成S1后不得自行恢复I。

S1完成会解除I。S1失败或范围扩张时I保持paused，不把未知Runtime问题转交Internals consumer修补。

## 2. Existing capability and expected owner

先验证最短闭环：

```text
admitted RuntimeAssembly raw HTTP gateway
  -> concrete Host request entry/response sink
  -> test-service wrapper
  -> PackageDirect stream producer
  -> existing StreamRuntime registry
  -> raw HTTP stream events
```

当前预期测试owner：

```text
runtime/host/src/loader/assembly_admission/tests/execution/**
runtime/host/src/host/router_session/tests/runtime_assembly_request.rs
```

当前可能的production owner只限现有association/lifetime链：

```text
runtime/eval/src/program_stream.rs
runtime/eval/src/program_execution.rs
runtime/host/src/capability_context/stream_runtime.rs
runtime/host/src/eval_capability_adapter/file_stream.rs
```

这些是预期owner，不是要求无条件修改的文件白名单。零worktree预检必须先确认真实create/lookup调用链、
现有兄弟ownership和最小fixture位置；只有稳定RED证明因果关系后才可修改直接相关production文件。

## 3. Required RED and diagnosis

第一笔test修改必须增加一个真实交叉fixture：

- 使用linked/admitted assembly和concrete Host request entry；
- gateway entry为`rawHttp`；
- handler是wrapper，wrapper迭代或转发一个真实`PackageDirect` stream producer；
- response走真实Host HTTP gateway response sink，不能直接调用handler、手工构造Interpreter、替换
  stream registry或使用mock response sink；
- 同一fixture记录每个相关stream在create/register与lookup/next时的：
  - registry identity；
  - request generation；
  - stream id。

诊断trace可以是task-local临时instrumentation，但不能增加public API，最终必须撤回；最终测试只能保留
行为断言或已有内部可观察量。必须在同一未修改production candidate上重复得到相同失败与身份轨迹，形成
稳定RED。I checkpoint的错误文本、静态推断或只覆盖其中一半形状的旧fixture不能替代本RED。

稳定RED后只按轨迹修复现有registry association或owner lifetime。result必须同时记录：

- create与lookup是否使用相同registry identity/request generation/stream id；
- owner何时打开、clone、捕获、关闭request scope；
- 第一个偏离既定语义的production symbol；
- 修复前后活动stream归零及single-terminal证据。

不得把current diagnosis预写为第二个registry、owner过早drop、heap差异、alias/linker或任何其它已知根因。

## 4. Completion matrix

同一concrete Host/raw HTTP fixture族至少闭合：

| 维度 | 必须证明 |
| --- | --- |
| return stream | wrapper消费/转发`PackageDirect`返回的stream，完整items与normal end可见 |
| stream parameter | `PackageDirect` callable接收同request已有stream参数并顺序消费 |
| nested | 两层producer/forwarding保持同一request registry association |
| complete | 正常结束后producer、deferred entry、scope与活动stream全部清理 |
| producer error | 已发items保留，error只终止一次且registry清理 |
| consumer break | stop向producer传播，晚到item不写HTTP response，registry清理 |
| request cancel | gateway/client cancellation停止ancestor producer并清理owner |
| effect stream | `TestEffectCaseContext` wire snapshots只在HTTP child当前runtime生成stream |
| local stream | handler本地producer保持GREEN，防止修复破坏普通raw HTTP stream |
| service-call non-regression | server stream仍按既有boundary materialization执行，不共享package-local registry |

参数和return属于同一package/local request内的直接Stream面；container字段、持久化、actor boundary和新的
service ABI不在本任务内。

## 5. Prohibitions and stop conditions

禁止：

- 新增第二个registry、跨request共享registry或测试专用stream bridge；
- 新协议、header、token、wire/schema/artifact代际；
- compiler、Router、test-runner、std或Internals production修改；
- 让HTTP parent的stream handle进入child；
- 让service call绕过boundary materialization；
- 为得到GREEN修改I测试语义、删除失败case或依赖网络chunk边界；
- 无稳定RED时修改production。

若稳定RED无法建立、身份轨迹没有单一偏离点、修复要求多个新机制、需要上述禁止owner，或一次有界预检后
仍有多个会改变实现方向的未知量，立即返回`TASK_NOT_EXECUTABLE`或`TASK_SCOPE_EXPANDED`。报告fixture、
轨迹、遮挡关系、已完成提交与最小后继，不继续猜测。

## 6. Evidence owner

S1开发Agent唯一拥有以下聚焦证据；selector名可以按最终fixture命名机械调整，但result必须记录实际命令：

```text
cargo test --locked -p skiff-runtime-host package_direct_http_stream_registry -- --nocapture
cargo test --locked -p skiff-runtime-host \
  typed_execution_package_direct_stream_installs_exact_producer_context_full_chain -- --nocapture
cargo test --locked -p skiff-runtime-host typed_execution_service_stream -- --nocapture
cargo test --locked -p skiff-runtime-host host_http_gateway -- --nocapture
cargo check --locked -p skiff-runtime-eval -p skiff-runtime-host
cargo fmt --all -- --check
git diff --check
```

不运行full workspace、J生态gate、stable/live/network/Mongo/OAuth/browser。production改动会使T/I/X/J中
Runtime/Host相关证据失效；S1合流后先运行便宜combined fixture，再恢复I。

## 7. Handoff

提交task implementation与result，报告：

- branch、worktree、implementation/result commit/tree；
- 实际production/test/doc写集；
- 稳定RED与GREEN身份轨迹；
- 上述矩阵逐项证据及反向搜索；
- 未运行的昂贵gate；
- `S1_COMPLETE`、`I_RESUME_UNBLOCKED`与scope状态。

交给`/root/phase05_integration_steward`串行集成、便宜探针和一级worktree/branch清理；不得自行写
integration、merge、push或恢复I。
