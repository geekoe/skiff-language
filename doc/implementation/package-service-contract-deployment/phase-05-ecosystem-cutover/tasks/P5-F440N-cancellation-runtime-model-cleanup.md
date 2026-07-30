# P5-F440N Cancellation runtime platform-error model cleanup

状态：Ready。对应 F439A 冻结 DAG 的 **M0**；R1、R2 已完成。

## 直接父节点

- `P5-F439A-cancellation-public-surface-owner-audit-result.md`
- `P5-F440I-cancellation-native-eval-service-channel-result.md`
- `P5-F440K-cancellation-request-host-transport-finalization-result.md`

实现基线为 `aa14721be58646492a84ea7541a0a1d3a197ca01`
（tree `7f145203fa5f620cddc1911818278e109ac619ac`）。

F440I 已使 native/eval/service channel 对 legacy cancellation identity fail closed；F440K 已使
request/Host/transport 不再产生 cancellation payload、catch 或普通 response。当前剩余 public owner
只有 runtime model 的 finite platform-error registry。

## 目标与唯一写集

从 runtime model 完全删除 cancellation 的公开 platform error identity：

- 删除 `PlatformBuiltinErrorIdentity::Cancel`；
- 删除 serde spelling `CancelError`；
- 删除 `from_symbol("CancelError")`、`symbol()` 与由该 variant 派生的 catch identity；
- legacy `PlatformError` envelope 的 `builtinErrorIdentity: "CancelError"` 必须严格反序列化失败；
- `TimeoutError` 及其它现有 platform identities 的 symbol、serde、catch 与 envelope round-trip
  保持不变。

唯一 production/test 写集：

- `runtime/model/**`
- 本 leaf result

禁止修改 native、eval、request、Host、transport、Router、compiler、artifact、scripts、fixtures、
其它 task/result或权威设计。不得派子 agent。

## 测试先行

先在 `runtime/model` 添加真实 red，至少覆盖：

1. 精确 legacy JSON：

   ```json
   {
     "kind": "platformError",
     "builtinErrorIdentity": "CancelError",
     "encodedPayload": [],
     "traceId": "trace-cancel",
     "errorId": "error-cancel"
   }
   ```

   反序列化为 `ServiceErrorEnvelope` 必须失败；旧实现应先证明会接受。
2. `PlatformBuiltinErrorIdentity::from_symbol("CancelError")` 返回 `None`。
3. 直接把 JSON string `"CancelError"` 反序列化为 finite enum 必须失败。
4. `TimeoutError` 仍可完成 enum serde、symbol、catch identity和完整
   `ServiceErrorEnvelope::PlatformError` round-trip。

负例中可以保留字符串字面量 `CancelError`，但不得保留可构造的 enum member、compatibility alias、
fallback 或 unknown-to-internal 降级。

## 验证

必跑：

```bash
cargo test -p skiff-runtime-model
cargo check -p skiff-runtime-model
cargo check -p skiff-runtime-eval
cargo check -p skiff-runtime-request
cargo check -p skiff-runtime-host
cargo check -p skiff-runtime-transport
cargo check -p runtime
cargo fmt --all -- --check
git diff --check
```

Cargo 命令统一使用：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

反向搜索：

```bash
rg -n 'PlatformBuiltinErrorIdentity::Cancel|Self::Cancel|serde\(rename = "CancelError"\)' \
  runtime/model
rg -n 'CancelError' runtime/model
```

第一条必须为零。第二条只允许命名清楚的 legacy rejection test 字符串，并在 result 逐项列出。补充确认
`CancelError|PlatformBuiltinErrorIdentity::Cancel` 在非文档 production 根中没有重新出现。

## 停止与交付

若删除 variant 后仍有 runtime/model 外的 production consumer，记录精确路径并返回
`TASK_SCOPE_EXPANDED`；不得越界修改。普通 compile/test fixture 漂移也不能扩张本 leaf。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440n-cancellation-model`
- branch：`codex/p5-f440n-cancellation-model`
- result：`P5-F440N-cancellation-runtime-model-cleanup-result.md`

Implementation 与 result 分开提交；不 merge/rebase/push，不访问 stable/live。
