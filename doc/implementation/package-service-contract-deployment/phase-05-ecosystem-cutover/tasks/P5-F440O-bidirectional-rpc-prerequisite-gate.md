# P5-F440O Bidirectional WebSocket RPC prerequisite gate

状态：Ready。只读 checkpoint gate；对应 F440B DAG 的 **P0**。

## 直接父节点

- `P5-F440B-bidirectional-websocket-owner-audit-result.md`
- `P5-F440M-external-manifest-identity-deployment-follower-result.md`
- `P5-F440K-cancellation-request-host-transport-finalization-result.md`
- `P5-F440N-cancellation-runtime-model-cleanup-result.md`
- `P5-F440J-cancellation-router-pending-projection-result.md`

验收输入为 `d31b4e7f28ef415c61c9e4ada2a1168703d4adcf`
（tree `e55dd42041e0b1f84f233873d221586b6815a286`）。

## 目标

在启动 T0 前，确认两个 shared prerequisite 同时存在于同一代码状态：

1. external manifest / JSON-RPC authoring、artifact identity和deployment follower已完整合流；
2. cancellation public surface已在 compiler/artifact、capability/native/eval、request/Host/transport、
   Router和runtime model按已分配 owner硬切，control cancellation仍保留。

本任务不实现、不修 fixture、不解释新设计。只建立精确 commit/tree、可执行聚焦证据和已知 follower
blocker inventory。若 gate 失败，返回最早失效 owner；不得现场修复。

## 唯一写集

- 本 leaf result

禁止修改 production、test、fixture、权威设计或其它 task/result。不得派子 agent，不访问 stable/live。

## 验收

先确认所有直接父 implementation/result commit 都是输入 HEAD ancestor，并记录 exact current tree。

至少运行：

```bash
cargo test -p skiff-artifact-model gateway
cargo test -p skiff-artifact-identity gateway
cargo test -p skiff-artifact-identity deployment
cargo test -p skiff-deployment
cargo test -p skiff-compiler --test websocket_ingress
cargo test -p skiff-runtime-model
cargo test -p skiff-runtime-capability-context
cargo test -p skiff-runtime-transport
pnpm --dir router test -- cancellation
cargo fmt --all -- --check
git diff --check
```

Cargo 命令统一使用：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

Router selector必须先列出/确认实际非零测试；若命令形态会展开全套，改用当前 package script支持的精确
file/name selector并在 result记录。不得安装依赖；若既有依赖不可用，记录环境 blocker。

补充只读检查：

```bash
rg -n 'CancelError|PlatformBuiltinErrorIdentity::Cancel' \
  artifact-model compiler runtime router --glob '!**/README.md'
rg -n 'WebSocketJsonRpc|websocketJsonRpc' artifact-model artifact-identity deployment compiler
```

第一条允许明确的 legacy rejection test/fixture和 `request.cancel` 控制语义中的普通单词
`cancel`，不允许 production公开 error identity；逐项分类。第二条必须覆盖 authoring、artifact
surface、identity/deployment validation与compiler projection，不以计数代替 owner检查。

当前 runtime/eval/Host 可能因尚未实施的 JSON-RPC consumer match arm而 compile-fail；若首错精确位于
F440B 已分配给 T0/E0/R0 的 follower，记录为后继输入，不把它误判为 prerequisite 回归。若首错来自
已合流 checkpoint自身或仍有 production `CancelError` consumer，则 gate FAIL。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f440o-rpc-prerequisite-gate`
- branch：`codex/p5-f440o-rpc-prerequisite-gate`
- result：`P5-F440O-bidirectional-rpc-prerequisite-gate-result.md`

只提交 result；不 merge/rebase/push。
