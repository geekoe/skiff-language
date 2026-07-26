# P5-F419 Suspension runtime unified boundary

状态：Ready（N3）。

## 直接父节点

- `P5-F416-suspension-schema-identity-current-checkpoint-result.md`

需要核对 runtime current调用链、F415 fixture debt或跨节点负矩阵时，再沿父节点引用读取
`P5-D93-suspension-current-base-reconciliation-audit-result.md`。

## 精确起点与任务边界

- integrated N0 checkpoint：
  `c597e3c0e5ecb9d1711b1a25a2660ea9cc972a60`；
- N0 implementation：
  `57d0a5551aaa62e5a71655050478c1447f94324d`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时必须证明三个 commit 都是 HEAD ancestor。本节点可与 F417 / F418并行；当前是实现检查点，不宣称
workspace或ecosystem稳定。

独占 production 写入范围：

```text
runtime/**
本任务 result
```

核心 owner：

```text
runtime/capability-context/src/execution_control.rs
runtime/request/src/execution_budget.rs
runtime/request/src/execution_control.rs
runtime/host/src/eval_capability_adapter/execution.rs
runtime/model/src/callback_projection.rs
runtime/eval/src/assembly_execution/{mod.rs,ordinary.rs,async_stream_cancel.rs,callback_native.rs,
  websocket_contract_plan.rs,projection.rs,boundary_materialization/tests.rs,ordinary/tests.rs,
  ordinary/tests/service_error_consumer.rs,ordinary/tests/source_inline_effect_e2e.rs,
  service_error_channel/tests.rs}
runtime/native/src/callback_adapter.rs
runtime/{boundary,linker,loader,host,package-test}/**/*fixture*
```

禁止修改 artifact-model、artifact-identity、compiler、deployment、router、scripts、test-runner、
cross-system fixture、ecosystem source或设计；不得派子 Agent。

## 必须实现的终态

### 1. 统一 service boundary lane

- Unary service call无条件进入 `async_stream_cancel::execute_service_call` 所在统一 boundary lane。
- ServerStream继续使用同一模块；Unsupported stream保持 typed error。
- `ordinary.rs` 只保留 package-direct executor，删除 service executor与
  `validate_ordinary_operation`。
- provider第一次 poll若 Ready，可在同一次 poll返回；保守 caller effect不等于强制 yield。
- callback与WebSocket只删除 provider summary equality，仍严格验证 shape、target与ABI。
- HTTP gateway 的 concrete executable / Package callable summary检查全部保留。

### 2. Cancel、deadline与provider竞争

给 `ExecutionControlApi`、borrowed与owned execution-control API增加只读：

```rust
deadline() -> Option<std::time::Instant>
```

request implementation、Host adapter与test double逐跳精确转发。等待 pending provider时使用
`tokio::time::sleep_until`，unary、stream item、stream terminal与publication wait都必须同时观察：

```text
ancestor/request cancellation
deadline
provider future
```

使用 biased priority：

1. cancellation；
2. 已到期 deadline；
3. provider结果。

因此 cancel与deadline同时ready时cancel优先。deadline必须返回既有 typed `DeadlineExceeded`，同时
cancel provider request；不能降成 `Cancelled`。detached stream持有
`OwnedExecutionControl`，不能只保存 raw token。timeout / cancellation后stream task与lease必须归零。

### 3. 删除旧 protocol summary consumer

- 删除 `BoundaryOperationContract.may_suspend`、cancellation和
  `CallbackContractOperationProjection.may_suspend` 的 runtime copy、accessor、branch与fixture。
- 不得删除 concrete executable、Package callable、actor/native/builtin summary。
- 不得给 contract/schema重新增加默认位或兼容路径。

## F415 mapping fixture与production preservation

D93确认 exact 13个缺失 initializer：

| file | 数量 |
| --- | ---: |
| `runtime/eval/src/assembly_execution/ordinary/tests/service_error_consumer.rs` | 4 |
| `runtime/eval/src/assembly_execution/ordinary/tests/source_inline_effect_e2e.rs` | 3 |
| `runtime/eval/src/assembly_execution/ordinary/tests.rs` | 4 |
| `runtime/eval/src/assembly_execution/service_error_channel/tests.rs` | 2 |

同一 dependency edge的 requirement与binding必须携带相同显式
`collection_name_mapping`；确实无mapping的 fixture才显式写 empty。不得给model struct增加 Rust
default或删除字段。

以下 production mapping owner及其 exact validation / projection必须保持：

```text
runtime/linked-program/src/shared_image.rs
runtime/linker/src/assembly.rs
runtime/loader/src/runtime_assembly/graph_validation.rs
runtime/host/src/loader/active_assembly_context.rs
```

mapping drift、unknown source、collision与ambiguous active edge继续fail closed。

## 验收矩阵

正例至少证明：

- Ready unary同poll返回；
- Pending provider分别被provider、cancel、deadline唤醒；
- cancel/deadline同时ready时cancel优先；
- deadline产生 typed DeadlineExceeded并发出provider cancel；
- stream item、terminal与publication wait都覆盖cancel/deadline；
- callback/WS summary差异但shape一致时接受；
- exact 13个mapping initializer编译并保持逐跳值。

负例至少证明：

- callback/WS shape、target或ABI mismatch仍拒绝；
- deadline不变成Cancelled；
- timeout后必须观察到provider cancel signal；
- task / lease没有泄漏；
- Unsupported stream/callback保持 typed error；
- mapping drift/collision validators仍拒绝。

## 验证与交付

先用相同 selector加 `-- --list` 记录实际数量，再运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-capability-context execution_control
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-request execution_budget
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-model callback_projection
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-eval assembly_execution
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-native callback_adapter
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-linker assembly
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-loader runtime_assembly
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-host assembly_admission
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo check --locked \
    -p skiff-runtime-capability-context \
    -p skiff-runtime-request \
    -p skiff-runtime-model \
    -p skiff-runtime-eval \
    -p skiff-runtime-native \
    -p skiff-runtime-linker \
    -p skiff-runtime-loader \
    -p skiff-runtime-host
cargo fmt --all -- --check
git diff --check
```

D93 listing基线为 capability `1`、request `5`、model `3`、native `7`、linker `30`、loader `17`、
host `30`；eval当时被13个initializer编译错误遮挡，本节点必须让完整
`assembly_execution` selector能够列出并通过。以当前实际 listing为准并记录变化。不要运行
workspace/full isolated/stable/live。

写 `P5-F419-suspension-runtime-unified-boundary-result.md`，记录 exact commit/tree、unified lane数据流、
poll/cancel/deadline优先级、typed timeout与provider cancellation证据、callback/WS检查保留项、13个mapping
修复、实际测试计数和所有未运行项。提交并保持 clean；不 merge/rebase/push。

若一次有界探查后发现必须越过授权 production root、公共契约仍不明确或任务实际拆成多个新 owner，停止并返回
`TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`，不要自行扩大范围。
