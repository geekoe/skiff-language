# P5-F445H-I7-P1T Activation client timeout

状态：`READY_FOR_IMPLEMENTATION`。

## 1. Purpose

本任务承接P1D已经冻结的三预算域，只修复test-runner activation control HTTP client仍复用普通
business request `30s` deadline的问题。

## 2. Frozen baseline and ownership

| 项 | 值 |
| --- | --- |
| Skiff baseline commit | `564636a557c638d1b21b66fcc3394ea076243ff2` |
| Skiff baseline tree | `22b6089fc0ce22358ea28aa590bd2f01bb6caeba` |
| leaf branch | `codex/p5-f445h-i7-p1t-activation-client` |
| leaf worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-p1t-activation-client` |
| integration owner | `/root/phase05_integration_steward` |

零worktree预检确认，test-runner只有一个`HTTP_TIMEOUT = 30s`，同时供activation control request与普通
test dispatch使用。拆分可以完全留在test-runner私有runtime execution边界内，不需要公共API或设计扩张。

## 3. Frozen implementation

1. Activation control HTTP client使用独立`150000ms` deadline。
2. 该值严格大于Router缺省`activation.prepareTimeoutMs = 120000`，因此缺省组合由Router先裁决prepare
   timeout，client不会在此前断开。
3. 普通test dispatch HTTP client保持`30000ms`。
4. 禁止把一个共享全局timeout整体调大。
5. Deadline计算溢出沿用`CanonicalFixtureError::InvalidInput` fail closed。
6. Skiff未发布，不保留旧alias、fallback或dual path。

## 4. Write scope and validation

写集严格限：

```text
test-runner/src/runtime_execution.rs
test-runner/src/runtime_execution/tests/orchestration.rs
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-P1T-activation-client-timeout.md
  P5-F445H-I7-P1T-activation-client-timeout-result.md
```

验证必须包含真实RED到GREEN、两类预算值与call site结构测试、overflow负例、test-runner完整selector、
Rust check/fmt、反向搜索和`git diff --check`。禁止修改Router、Host/runtime、scripts或Internals；禁止
stable/live/network/Mongo/OAuth/browser。
