# P5-F232 Formal-indexed write and escape transfer result

状态：完成。

## 实现

Callable-effects transfer 现在为 caller write 记录精确 formal parameter selector，并为
每条 escape lane 分别记录 formal parameter selector。selector 仅存在于 compiler 内部：

- field/receiver mutation 将被写 graph 的 formal 记录为 write selector；`push`、`set`
  和 `delete` 的 receiver 是 formal 0；
- 源码 `emit(value)`、database storage 及已审计的 response-stream native 分别从
  value provenance 记录 Stream、Database 与 External lane selector；
- local helper application 只将 write 与每条 escape lane 映射到被 selector 选中的
  actual；nested helper 与递归 SCC 固定点使用相同 lattice join；
- aggregate effect 缺失 selector 时仍检查全部 actual。unknown、dynamic、external 和
  dependency artifact summary 因而继续 fail closed，没有被抑制。

public artifact ABI 没有新增 selector。跨 package callable 目前只携带既有 aggregate
effect/provenance summary，应用时仍按 unscoped effect 保守处理；F232 的真实 false
attribution 均在同 package private helper 链内，因此没有可证明需要序列化的新 identity。
boundary eligibility filter 未修改。

## 测试

新增 compiler/source 正负探针覆盖：

- `add(headers, request)`：Fresh headers 与 caller request 不产生 caller write；
  caller headers 保留 write 与 same-heap；
- nested helper 与 fixed-point recursion 中保持相同 write 结论；
- `forward(stream, state)`：Fresh stream 与 caller state 不产生 caller-value Stream
  escape；caller stream 保留 Stream escape；
- nested helper 与 fixed-point recursion 中保持相同 escape 结论。

既有 suite 同时继续覆盖 unknown/dynamic/unattributed fail-closed、Database lane、
receiver/native semantics 和 detached-boundary filtering。

验证通过：

- `cargo test -p skiff-compiler-source --lib --no-fail-fast`：271 passed；
- `cargo check --workspace`；
- `cargo fmt -p skiff-compiler-source`；
- `git diff --check`。

workspace 全局 `cargo fmt --all -- --check` 仍会报告 integration baseline 中三个与本任务
无关的预存格式差异；本任务四个 source 文件已经过 package-scoped rustfmt。

## 真实 Relay

使用 `/Users/geek/workspace/internals-p5-f188/codex-relay/service`、本 worktree compiler
以及保留的隔离 artifact store `/tmp/skiff-f227-relay.iB6dOG` 构建；没有操作 shared
stable instance。新 package build：

```text
skiff-package-build-v4:sha256:6cc37cd4074fa0c0a6ad7a183fdb6157444da83bc084de4d286147349edef3cf
```

`v1Proxy` 已由 F230 后的 write/Stream residual rejection 变为 Available，证明本任务
两条真实 local-helper false attribution 均已闭合。17 个 intended operations 当前为
15 Available / 2 Unavailable。

剩余两项已独立定位为 completed unary 链：

- `relayProxy.responsesCompleted`：UnknownCallTarget provenance，并保留 unknown、
  write、return alias、throw alias 与 same-heap rejection；
- `relayProxy.responsesCompletedResult`：UnknownCallTarget provenance，并保留 unknown、
  write、throw alias 与 same-heap rejection。

两者均进入 cross-package
`llmProviders/chatgptPlan.responsesCompleted(scope, request)` 链；dependency artifact
summary 不携带本任务的 internal selector，按设计继续 aggregate fail-closed。它们不是
`v1Proxy` 的 local receiver-write/Stream-lane selector 问题，后续若要精化必须单独决定
canonical cross-package selector serialization 与 identity coverage。

首次运行 internals 主 worktree 的 isolated-graph helper 因脚本版本不匹配在构建前拒绝；
随后任务指定的 `internals-p5-f188` 全图 helper又因 aihub 旧 `config.dev.yml` schema 在
Relay 前拒绝。最终直接使用同一隔离 dependency store 完成真实 Relay package build 与
17 项 projection 验收。

没有 push、没有 stable 操作，也没有清理共享磁盘状态。
