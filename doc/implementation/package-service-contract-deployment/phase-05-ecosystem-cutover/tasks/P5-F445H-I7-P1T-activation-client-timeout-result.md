# P5-F445H-I7-P1T Activation client timeout result

状态：

```text
PASS
P1T_COMPLETE = YES
ACTIVATION_CLIENT_TIMEOUT_SPLIT = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```
## 1. Exact input and scope

| 项 | 值 |
| --- | --- |
| baseline commit/tree | `564636a557c638d1b21b66fcc3394ea076243ff2` / `22b6089fc0ce22358ea28aa590bd2f01bb6caeba` |
| branch | `codex/p5-f445h-i7-p1t-activation-client` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-p1t-activation-client` |
| integration owner | `/root/phase05_integration_steward` |

最终result commit/tree由Git handoff记录；result文档不自引用自身commit identity。实际写集只有
test-runner runtime execution、其直接单测以及P1T task/result，没有修改Router、Host/runtime、scripts、
fixtures或其它repo。

## 2. Implementation result

- Activation control HTTP request使用独立`ACTIVATION_HTTP_TIMEOUT = 150s`。
- 普通package-test dispatch保持`BUSINESS_HTTP_TIMEOUT = 30s`。
- 结构测试固定两个call site各自只消费对应预算，并禁止退回共享`HTTP_TIMEOUT`。
- `150000ms > 120000ms`由精确单测固定，默认组合会先由Router裁决prepare timeout。
- Deadline使用`Instant::checked_add`；overflow保持
  `CanonicalFixtureError::InvalidInput("HTTP deadline overflow")` fail closed。

## 3. RED to GREEN evidence

RED先只增加预算、call site与overflow测试。编译按预期失败，报告
`ACTIVATION_HTTP_TIMEOUT`、`BUSINESS_HTTP_TIMEOUT`和`deadline_after_from`不存在。

GREEN实现拆分后：

| 检查 | 结果 |
| --- | --- |
| runtime execution聚焦单测 | PASS，`9 passed` |
| `node scripts/verify.mjs --only test-runner` | PASS；lib `44 passed / 2 ignored`，bootstrap `1 passed`，contract deployment `28 passed / 1 ignored` |
| `cargo check --manifest-path test-runner/Cargo.toml` | PASS；仅继承的compiler-source warning |
| `cargo fmt --manifest-path test-runner/Cargo.toml -- --check` | PASS |
| `git diff --check` | PASS |
| timeout/call site反向搜索 | PASS；activation `150s`、business `30s`，无`deadline_after(HTTP_TIMEOUT)` |

没有运行stable/live/network/Mongo/OAuth/browser，也没有push。

## 4. Handoff

本任务只完成test-runner activation client budget拆分。Router
`activation.prepareTimeoutMs`解析与coordinator裁决由P1其它owner负责；P1T没有声称其已完成。

```text
P1T_COMPLETE = YES
ACTIVATION_CLIENT_TIMEOUT_SPLIT = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```
