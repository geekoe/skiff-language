# P5-F440N Cancellation runtime platform-error model cleanup result

状态：`COMPLETED`。没有触发 `TASK_SCOPE_EXPANDED`。Runtime model 的 finite platform-error
registry 已彻底删除 cancellation identity；legacy `CancelError` symbol、enum JSON 和完整
`ServiceErrorEnvelope::PlatformError` 均 fail closed。`TimeoutError` 的 serde、symbol、catch
identity 与完整 envelope round-trip 保持不变。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| 任务指定 implementation 基线 | `aa14721be58646492a84ea7541a0a1d3a197ca01` | `7f145203fa5f620cddc1911818278e109ac619ac` |
| task worktree 起点 | `64894b9a2001c6a00a1cb6b67a1b2f350ec93a68` | `e593386d5fd4214b65e1c46596702ab069ca8d72` |
| implementation | `d435ea95994173b0dcfc11d5478b9d1c57b37454` | `92dea6336dc90a1de0525cc0c47b48da7a619683` |

基线到 task 起点只新增本任务文件。Implementation 只修改
`runtime/model/src/service_error.rs`；除此之外只新增本文 result。

## 2. 实现结果

- 删除 `PlatformBuiltinErrorIdentity::Cancel` 及其 `CancelError` serde spelling。
- 删除 `from_symbol("CancelError")` 映射和 `symbol()` 的 cancellation 分支；因此也不再有由该
  enum member 派生的 catch identity。
- 精确 legacy platform envelope 在 finite enum decode 阶段以 unknown variant 失败，不会因
  `encodedPayload: []` 的通用校验碰巧失败后被误认为 identity 已退休。
- 直接 enum JSON string `"CancelError"` 严格反序列化失败，symbol lookup 返回 `None`。
- `TimeoutError` 继续完成 enum serde、symbol lookup、catch identity和完整 platform envelope
  round-trip。

没有 compatibility alias、fallback、unknown-to-internal 降级或可构造的 legacy enum member。

## 3. 测试先行与验证

所有 Cargo 命令均使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

### 3.1 Red / green

Production 修改前运行 `cargo test -p skiff-runtime-model`：

```text
85 passed; 3 failed
```

三个真实 red 分别证明：

1. 精确 legacy envelope 虽最终因空 payload 失败，但旧 finite enum 已接受
   `CancelError`；测试要求它必须先以 unknown variant 被拒绝。
2. `from_symbol("CancelError")` 旧实现返回 `Some(Cancel)`。
3. 直接 enum JSON `"CancelError"` 旧实现可成功反序列化。

同一最终 implementation tree 的 green：

```text
cargo test -p skiff-runtime-model
88 passed; 0 failed; 0 ignored
doc-tests: 0
```

其中四个直接契约测试全部通过：三个 legacy rejection 和一个完整 Timeout 正例。

### 3.2 必跑命令

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-model` | PASS：88 passed |
| `cargo check -p skiff-runtime-model` | PASS |
| `cargo check -p skiff-runtime-eval` | 外部基线 blocker：3 个 `E0004`，见 3.3 |
| `cargo check -p skiff-runtime-request` | 被同一 eval dependency blocker 阻断 |
| `cargo check -p skiff-runtime-host` | 被同一 eval dependency blocker 阻断 |
| `cargo check -p skiff-runtime-transport` | PASS |
| `cargo check -p runtime` | 被同一 eval dependency blocker 阻断 |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

### 3.3 Consumer check 的既有 blocker

Eval、request、Host 与 driver check 共同停在 task 起点已经存在且本 leaf 未修改的 gateway
consumer 漂移：

```text
runtime/eval/src/runtime_http_gateway.rs:85
GatewayAdapterKind::WebSocketJsonRpc not covered

runtime/eval/src/runtime_http_gateway.rs:439
GatewayAdapterSource::{WebSocketJsonRpcParams, WebSocketBusinessIdentity} not covered

runtime/eval/src/runtime_websocket_connect.rs:171
GatewayAdapterSource::{WebSocketJsonRpcParams, WebSocketBusinessIdentity} not covered
```

这三个 `E0004` 与 cancellation model 删除无关；对应文件不在 implementation diff 或本任务写集。
删除最后一个 cancellation registry member 后，
`runtime/eval/src/assembly_execution/service_error_channel.rs:1433` 的旧兜底分支另产生一个
`unreachable_patterns` warning，但不是 compile error，也不是残留 cancellation identity。

## 4. Reverse search 与 consumer inventory

任务要求的精确 model 搜索：

```text
rg -n 'PlatformBuiltinErrorIdentity::Cancel|Self::Cancel|serde\(rename = "CancelError"\)' \
  runtime/model
```

结果：`ZERO_MATCHES`。

宽 model 搜索 `rg -n 'CancelError' runtime/model` 仅有以下四行，全部位于命名清楚的 legacy
rejection tests：

| 路径 | 分类 |
| --- | --- |
| `runtime/model/src/service_error.rs:594` | 精确 legacy envelope 的 wire spelling |
| `runtime/model/src/service_error.rs:602` | 断言 serde 必须报告 unknown legacy variant |
| `runtime/model/src/service_error.rs:610` | legacy symbol lookup 返回 `None` |
| `runtime/model/src/service_error.rs:617` | legacy enum JSON string 反序列化失败 |

仓库级非文档反向搜索没有发现可构造的 production cancellation platform identity。现有其它
`CancelError` spelling 均是负向 owner：

- `artifact-model/src/file_ir.rs:101` 是既有 retired File IR admission tombstone；
- `runtime/eval/src/assembly_execution/projection.rs:699,704` 是 legacy linked spelling
  fail-closed test；
- artifact/compiler 其它匹配均为 retired spelling rejection tests。

唯一 runtime/model 外仍构造已删除 member 的 consumer 是 test-only
`runtime/eval/src/assembly_execution/service_error_channel/tests.rs:127`。它是任务明确禁止扩张
处理的 compile/test fixture drift，不是 production consumer，因此未触发
`TASK_SCOPE_EXPANDED`，本 leaf 未越界修改。

## 5. Scope 与禁令

- 没有修改 native、eval、request、Host、transport、Router、compiler、artifact、scripts、
  fixtures、其它 task/result或权威设计。
- 没有访问 live、stable、instance、watch、router、runtime 或 telemetry 进程。
- 没有 merge、rebase 或 push。
- Implementation 与 result 分开提交；result commit/tree由最终交付消息记录。
