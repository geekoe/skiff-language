# P5-F230 Fresh-local heap store analysis result

状态：完成。

## 实现

Callable-effects transfer 现在维护仅存在于 compiler 内部的 allocation-site
Fresh root、field-insensitive transitive heap payload 和 mutated-root 集合。直接局部别名
保留 root identity；field store 使用 weak update，因此 loop 固定点和 suspension 不会撤销
本地 ownership。

已知 local helper 通过 `parameter_stores` 记录 formal root 与写入 payload provenance；
callee/application transfer 将 formal 映射回 actual。Fresh actual 的写入留在 caller evaluator
heap，caller-owned actual 则精确产生 write 和 same-heap effects。root id 与 helper transfer
均不进入 artifact ABI。

Fresh root 写入 caller-derived reference 后，后续 field load、return、throw 或 escape 会看到
transitive payload；Fresh 不再隐藏 caller handle。以下路径继续关闭式失败：

- nested/ambiguous/unknown ownership field store；
- mutated Fresh root 进入 Map、Array 或 database storage；
- unknown/dynamic/external call；
- 既有 native、database、callback、spawn 与 stream escape gate。

拒绝的 heap store 使用新的 `UnsupportedHeapStore` provenance reason；compiler projection 和
deployment eligibility 仍将其映射为 `UnknownEffect`，没有放宽 boundary。

## 测试

`compiler/source` 覆盖：

- Fresh record reference/scalar field write与局部 alias；
- known helper 分别作用于 Fresh actual 和 caller-owned actual；
- caller-derived payload taint后 return；
- loop、真实 `std.time.sleep` suspension 以及 suspension 后 store；
- Relay-shaped 24-field state，经 alias 与多个 helper 写入；
- nested target、unknown RHS、Map、Array 和 DB negatives；
- SCC 初始 bottom 不被误判为 sticky heap-store failure。

验证通过：

- `cargo test -p skiff-compiler-source --lib --no-fail-fast`：269 passed；
- `cargo check --workspace`；
- `cargo fmt --all`；
- `git diff --check`。

## 真实 Relay

使用 `/Users/geek/workspace/internals-p5-f188/codex-relay/service` 和隔离 artifact store
`/tmp/skiff-f227-relay.iB6dOG` 构建；未操作 shared stable instance。最新 package build：

```text
skiff-package-build-v4:sha256:17d8957025213b89e4394b3c4d4ebf28775368c6441773a20f56cd5d5d211992
```

`v1Proxy` 的 provenance 已从 `Unknown(UnsupportedControlFlow)` 变为 `Analyzed`：

```text
invokesUnknownTarget: false
throwsCallerAlias: false
requiresSameHeapIdentity: false
returnsCallerAlias: false
maySuspend: true
escapeLanes: [stream, database]
```

精确 dependency target 仍是 preorder 189 的
`agine.ai/llm-providers:chatgptPlan.responses`。真实 stream suspension 和 stream/database
escape 没有被抹掉。

该 operation 仍因 `writesCallerReachable` 与 stream escape 保持 boundary unavailable。
本任务没有为了 Available 删除这些 effect；当前 artifact 的公开 callable 聚合并不足以把
剩余 write 精确定位到首个 private callee，因此将其保留为后续独立审计项。

没有 push，也没有操作 stable 或清理共享磁盘状态。
