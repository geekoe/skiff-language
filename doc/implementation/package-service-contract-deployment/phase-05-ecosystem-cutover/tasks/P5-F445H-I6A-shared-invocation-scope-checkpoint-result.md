# P5-F445H-I6A shared invocation-scope checkpoint result

状态：

```text
PASS
I6_A_COMPLETE = YES
I6_B_HTTP_UNBLOCKED = YES
I6_C_WEBSOCKET_UNBLOCKED = YES
I6_D_TIME_FILE_ACTOR_HOST_RESPONSE_SOURCE_UNBLOCKED = YES
TASK_SCOPE_EXPANDED = NO
```

本节点只建立 Rust 内部 invocation-time execution carrier，不实现 I6-B/C/D 的 lower wait、
transport deadline、request registry settlement、resource cleanup 或 lease winner。

## 1. 候选身份

| 项 | 值 |
| --- | --- |
| 固定父 base commit | `07392f1a1b01f3cafb27c7882b76e6646758444c` |
| 固定父 base tree | `675041dcea6c1f868ccdcd79f8e05b14a54be964` |
| 任务开始 HEAD | `c8dc205dcd691d1f1108ded1f6928379563f6c00` |
| 任务开始 tree | `ede7f229d56a81353f5d83dc44c30f930a7f93c3` |
| implementation commit | `abc75ec34999accdab5897fdae7ec8d201c2ba07` |
| implementation tree | `0700c2be91ccf32f223a7fc550e78332961d832b` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i6a-scope` |
| branch | `codex/p5-f445h-i6a-scope` |

开始后的第一处 production 修改精确位于
`runtime/host/src/eval_capability_adapter/execution.rs`，先补
`RuntimeExecutionControl` / `RuntimeOwnedExecutionControl` 的 full-scope façade；修改前没有运行
测试或开放式审计。

## 2. 实现与实际写集

implementation commit 只包含以下五个允许路径：

```text
runtime/host/src/eval_capability_adapter/execution.rs
runtime/eval/src/native_capability.rs
runtime/eval/src/capabilities.rs
runtime/capability-context/src/stream.rs
runtime/eval/src/program_execution/execution_scope_tests.rs
```

具体结果：

1. borrowed 与 owned Host adapter 都直接转发 concrete `ExecutionScope`；三条观察路径
   borrowed、owned、`owned.borrow()` 保留相同 deadline source/site/nesting、signals 与 lifecycle。
2. 两条 `derive_scope(...)` 都调用 concrete derive，并统一通过既有
   `From<ExecutionScopeDeriveError> for ExecutionScopeAccessError` 保留 `Derive` variant；没有从
   `deadline()` 或单一 token 反推 scope。
3. `RuntimeNativeCapabilityProjectionSource::new` 精确一次执行
   `context.execution().owned()`；`new_supervised` 复用 `new`，没有第二次 current-control read。
4. crate-private `RuntimeNativeInvocationExecutionControl` 用同一 `Arc<OwnedExecutionControl>` clone
   分发到 Actor、file、file source-stream、time、HTTP、HTTP response-stream 与 WebSocket native
   context。file 普通与 supervised projection 的两个拆分 consumer 均证明共享同一 carrier。
5. response-stream 使用同一次捕获的 owned control，替代原 getter 内第二次
   `context.execution()`；其既有 cancellation 行为不变。
6. time/HTTP/file/Actor/WebSocket 的旧 lower context 仍保留给各自后续 consumer 任务；本节点只附加
   current invocation carrier，没有提前把 lower wait 切到新 winner。

没有修改 fixture constructor、E4 assertion、actual-Pending、timeout、concurrent、program/source
stream、service owner、public std/native 签名、artifact/schema/compiler/router、Cargo manifest 或
`Cargo.lock`。

## 3. 真实 RED / GREEN

### 3.1 Host façade RED

加入聚焦断言后，将四个 façade override 恢复到父节点原始缺失状态，执行：

```bash
cargo test -p skiff-runtime-host f445h_i6_scope_adapter -- --nocapture
```

真实结果为 `2` 个测试、`1 passed / 1 failed / 301 filtered`；失败精确为：

```text
borrowed scope: Unavailable
```

恢复 borrowed/owned 的 `execution_scope` / `derive_scope` override 后，同一命令 GREEN：
`2 passed / 0 failed / 301 filtered`。

### 3.2 Native invocation RED

carrier 机械接点先保持父节点的冻结 time-context control，derived context 的 current scope 为
nesting `2`、旧 snapshot 为 nesting `1`。执行：

```bash
cargo test -p skiff-runtime-eval f445h_i6_native_invocation_scope -- --nocapture
```

真实结果为 `1` 个测试、`0 passed / 1 failed / 395 filtered`，断言精确为：

```text
left: 1
right: 2
```

source 构造切到唯一 `context.execution().owned()` 后，同一命令 GREEN：
`1 passed / 0 failed / 395 filtered`。

## 4. 最终验证

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-host f445h_i6_scope_adapter -- --list` | PASS；精确 `2 tests, 0 benchmarks`。 |
| `cargo test -p skiff-runtime-host f445h_i6_scope_adapter -- --nocapture` | PASS；精确 `2 passed / 0 failed`。 |
| `cargo test -p skiff-runtime-eval f445h_i6_native_invocation_scope -- --list` | PASS；精确 `1 test, 0 benchmarks`。 |
| `cargo test -p skiff-runtime-eval f445h_i6_native_invocation_scope -- --nocapture` | PASS；精确 `1 passed / 0 failed`。 |
| `cargo check -p skiff-runtime-host -p skiff-runtime-eval --locked` | PASS；只有仓库既有 warning。 |
| `cargo fmt --check` | PASS，无输出。 |
| `git diff --check` | PASS，无输出。 |

listing 与实际执行数量分别为 Host `2/2`、Eval `1/1`，均非零且一致。

没有运行完整 crate/stage gate，没有连接或启动 network、stable/live、MongoDB，也没有访问本机
stable instance。

## 5. 自验收矩阵

| 合同条款 | 代码/测试证据 | 结论 |
| --- | --- | --- |
| borrowed/owned full scope | `execution.rs` 四个 override；Host full-scope test覆盖 borrowed、owned、reborrowed | PASS |
| deadline owner 与 nesting | Host test覆盖 inner-earlier、outer-earlier、equal outer-owner；检查 source/site/nesting | PASS |
| ancestor/local signals 与 lifecycle | Host test分别触发 outer-local 与 request ancestor；Eval test比较 lifecycle 并检查 child-local signal | PASS |
| derive error不折叠 | `runtime_owned_execution_control` 使用既有 `From`；聚焦 test精确断言 `ExecutionScopeAccessError::Derive` | PASS |
| invocation只读一次 current control | `native_capability.rs` 只有一处 `context.execution().owned()`；supervised 构造复用 `new` | PASS |
| 所有目标 consumer 可取得 carrier | Actor、file/file-stream、time、HTTP、response-stream、WebSocket wrapper均持有同一 invocation carrier clone | PASS |
| 同一 projection共享 carrier | 普通/supervised file bundle都以 `Arc::ptr_eq` 证明 file 与 source-stream carrier identity相同 | PASS |
| 不新建 root、不反向停止 parent | carrier只 clone 既有 owned control；scope lifecycle snapshot保持为零，无 root/token reconstruction | PASS |
| Ready operation无虚假 suspension | implementation diff无 `acquire_lease`、timer、timeout或 yield；lifecycle snapshot保持 default | PASS |
| 不提前实现 B/C/D | native operation methods仍使用原 lower context；只新增 internal carrier accessor | PASS |
| E4 owner与公共语义不变 | 无 E4 fixture/assertion、actual-Pending、std/native/artifact/schema/Cargo diff | PASS |

## 6. 反向搜索

合同第一条搜索仍有六个旧 capability payload getter：

```text
actor_context
file_source_stream_context (普通/supervised各一处)
time_context
http_client_context
websocket_context
```

这些 getter 现在都在同一 constructor call 中同时接收 `self.invocation_execution.clone()`；它们只保留
后续 B/C/D 尚未迁移的 lower context，不再是 suspending consumer 的唯一 scope 来源。
response-stream 不再二次读取 `context.execution()`。

`rg -n "fn (execution_scope|derive_scope)" .../execution.rs` 精确为 `4` hits，分别覆盖 borrowed/owned
两套 `execution_scope` 与 `derive_scope`。

`$/cancelRequest|-32800|CancelError|yield` 全局为 `46` 个既有 hits；implementation diff新增命中为
`0`。implementation diff 对 `acquire_lease|tokio::time|timeout` 同样为 `0`，因此本节点没有新增
公开 cancel、peer cancel、yield、timer 或 lower timeout。

## 7. DAG 解除

共享 carrier 签名与 consumer accessors 已冻结，未发现需要公共 API、production owner 或新语义的
缺口。因此：

- I6-B HTTP：`UNBLOCKED`；
- I6-C WebSocket request：`UNBLOCKED`；
- I6-D time/file/Actor/Host response-source：`UNBLOCKED`。

三个节点只是解除依赖，尚未由本任务实现。service dependency/callee timeout、I6-J 与阶段验收仍不在
本节点完成范围。
