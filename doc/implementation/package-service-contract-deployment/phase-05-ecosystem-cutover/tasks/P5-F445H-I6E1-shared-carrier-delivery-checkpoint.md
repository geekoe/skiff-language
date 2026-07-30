# P5-F445H-I6E1 shared invocation carrier delivery checkpoint

状态：Ready。把 I6-A 的同一个 invocation-time `OwnedExecutionControl` 从 Eval native wrapper
机械交付到 HTTP、WebSocket request、time、file、Actor control/method/spawn 的内部
adapter/API；本节点只建立可编译接线和真实 receipt，不实现下层 pending winner。

## 直接父节点

- `P5-F445H-I6E-invocation-carrier-delivery-preflight-result.md`
- `P5-F445H-I6A-shared-invocation-scope-checkpoint-result.md`
- `P5-F445H-I6B-http-current-scope-result.md`
- `P5-F445H-I6C-websocket-request-current-scope-result.md`
- `P5-F445H-I6D-host-operation-current-scope-result.md`

## 固定输入

```text
task base commit  1617299b23a1ac29ea889c573b8c49acd83785d0
task base tree    6d55ebfdf245f6bd3291e4016d59a52b1231b57a

production base commit  1000d290ce9ebc3cd5a792cf01f27b5835496a2a
production base tree    90c69b694fb38c7ec544149aec3b87a3b632496c
```

两棵 tree 的 production差异必须为空；后一个 tree只比 production base增加 I6E preflight result。

## 冻结接口

1. 唯一 carrier是 I6-A 已有 `RuntimeNativeInvocationExecutionControl` 内持有的
   `OwnedExecutionControl`。本任务不得创建第二种 scope snapshot、root wrapper或全局 registry。
2. 内部 trait/context/adapter传递 owned control；允许 clone同一 owned control，不得在交付过程中
   acquire lease、derive deadline、创建 timer或改变错误/winner。
3. current scope只由 E2–E6 consumer在 operation真正开始时读取。E1 receipt只证明完整 carrier穿过
   seam并保持同一次 invocation/current scope身份。
4. `requestJsonToConnection(connectionId, method, value)`等公开 Skiff/native参数保持不变；新增参数只在
   Rust crate-private/internal capability API。
5. 不修改 artifact/schema/ABI、Router/wire、std源文件、公开 timeout/cancel/yield语义。

## 实现范围

### A. 唯一共同 owner

`runtime/eval/src/capabilities.rs`

- HTTP unary/body-stream/SSE 三个 native dispatch转发 invocation owned control。
- WebSocket request内部 trait与真实 delegation转发 owned control，公开三参数不变。
- time wrapper向 `NativeTimeCapability` owned-control getter提供同一 carrier。
- file direct/provider/source delegation转发同一 carrier。
- Actor control delegation转发同一 carrier。
- 只做 delivery；不得在本文件实现 pending race或结果投影。

### B. capability-context内部 API

```text
runtime/capability-context/src/http.rs
runtime/capability-context/src/file.rs
runtime/capability-context/src/actor.rs
```

- 为真实会挂起的 operation增加 `OwnedExecutionControl` 内部参数，并把它原样交给实现。
- HTTP只覆盖 unary、body-stream open、SSE open。
- file覆盖六个 direct/provider operation与source `next`。
- Actor覆盖 get-or-create、replace、find、remove、method与spawn实际内部入口。
- 不在这一层 acquire lease或等待。

### C. native time seam

```text
runtime/native/src/capability.rs
runtime/host/src/capability_context/native_projection.rs
runtime/native/src/dispatch/prepared_tests.rs
```

- `NativeTimeCapability`仅新增取得 owned invocation control的内部 getter。
- alternate impl与测试 fake机械跟随。
- 不修改 `dispatch/time.rs` sleep行为；E4拥有真正 pending race。

### D. Host adapter receipt

```text
runtime/host/src/eval_capability_adapter/http.rs
runtime/host/src/eval_capability_adapter/file_stream.rs
runtime/host/src/eval_capability_adapter/websocket.rs
runtime/host/src/eval_capability_adapter/actor.rs
runtime/host/src/eval_capability_adapter/factory.rs
```

- adapter/constructor接收并继续转发 owned control。
- E1允许为 receipt存储或观察 carrier，但不得替换旧 root-token/deadline waiter行为。
- factory caller只做编译所需机械跟随；不得在本节点清理旧 root snapshot。

### E. Actor method/spawn capture

```text
runtime/eval/src/actor_dispatch.rs
runtime/eval/src/actor_dispatch/prepared_operation.rs
runtime/eval/src/spawn_ops.rs
```

- method prepared operation捕获当前 owned control并传到 Host adapter。
- spawn statement捕获当前 owned control并传到 submit入口。
- 不改变 Actor segment切分、post-await checkpoint、30s primitive、late response fence或业务错误。

### F. fixture与真实 receipt

允许机械跟随：

```text
runtime/eval/src/assembly_execution/ordinary/test_runtime.rs
runtime/eval/tests/f445h_e4r_combined/capability_harness.rs
runtime/eval/src/actor_dispatch/prepared_operation_tests.rs
runtime/eval/src/spawn_ops/canonical_tests.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/actor_dispatch.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/file_create_from_stream.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/support.rs
```

真实 receipt唯一行为测试 owner：

```text
runtime/eval/src/program_execution/execution_scope_tests.rs
```

新增 `f445h_i6_carrier_delivery_receipt`，必须：

1. 建立带外层 request scope与内层 derived scope的真实 Eval execution；
2. 通过 native projection取得 HTTP、WebSocket、time、file、Actor context；
3. 让各内部 API调用到记录型 lower adapter/fake，而不是只读 wrapper字段；
4. lower receipt取得的 owned control与调用时 current scope具有相同signals、absolute deadline、
   clock与owner；
5. operation ready结束后 E1没有新增 active lease、timer或waiter。

若单个测试无法在不实现 consumer的情况下经过全部 adapter，可用同一 fixture的多个 case，但 selector
listing必须非零且每种能力至少一条 method receipt；result必须如实列出。

## Agent并行

父任务 Agent可以派最多三个边界互斥的实现子 Agent：

1. capability-context三文件；
2. Host adapter与native time机械跟随；
3. Actor method/spawn与fixture机械跟随。

子 Agent不得再委派。父 Agent必须亲自拥有 `runtime/eval/src/capabilities.rs`、统一签名、集成冲突、
receipt测试和最终验证。若共享签名未冻结，不得让子 Agent各自发明接口。

子 worktree必须从本任务 base建立；父 Agent集成后清理全部已合并子 worktree/分支。

## 禁止项

- 不实现 HTTP/WS/time/file/Actor下层 pending winner、timeout、cancel或cleanup。
- 不新增公开 lifecycle metadata、peer cancellation、第四个 WebSocket业务参数或语言`yield`。
- 不修改 E4 stream、DB state machine、HTTP ingress、Router、wire、compiler/artifact/std。
- 不引入新的 Cargo dependency或修改 lockfile。
- 不运行 full gate、stable/live/network/Mongo。
- 不 merge integration、rebase或push。

## RED / GREEN 与验证

实现前必须建立真实 method receipt RED；旧代码应因内部接口未转发 carrier而无法满足 receipt，不得只用
编译失败冒充全部行为证据。

最小命令：

```text
cargo test -p skiff-runtime-eval f445h_i6_carrier_delivery_receipt -- --list
cargo test -p skiff-runtime-eval f445h_i6_carrier_delivery_receipt -- --nocapture
cargo check -p skiff-runtime-capability-context -p skiff-runtime-native \
  -p skiff-runtime-eval -p skiff-runtime-host --locked
cargo fmt --check
git diff --check
```

listing必须非零并与execution数量一致。允许使用隔离 target；不得清理或改写共享 target。

反向核对：

- 五条链的 Eval wrapper不再在内部 delegation处丢弃 carrier；
- E1没有 `acquire_lease`、timer、deadline derive或新的 cancellation winner；
- public Skiff/native签名与artifact/wire无diff；
- production写集不超出本合同。

## 停止条件

若实现需要：

- 修改公开 Skiff/native参数、artifact/wire或新增依赖环；
- 在 E1中提前实现 consumer winner才能编译；
- 为某一能力创建第二个 carrier/side channel；
- 修改未列出的行为 owner且不是机械 caller跟随；

则提交 `TASK_SCOPE_EXPANDED` result并停止，不保留半套接口。单纯的机械 constructor/test fake跟随可在
result中列明，但新增 production owner必须停止。

## 提交与完成

分开提交：

1. implementation + tests；
2. `P5-F445H-I6E1-shared-carrier-delivery-checkpoint-result.md`。

result必须写明：

- implementation commit/tree；
- 每条能力实际 carrier path与receipt；
- RED/GREEN和非零计数；
- 四 crate check、fmt、diff结果；
- 实际写集与禁止项反查；
- 子 Agent/worktree清理；
- `E2_E3_E4_E5_E6_UNBLOCKED = YES/NO`。

完成时父 worktree必须clean，不得push。
