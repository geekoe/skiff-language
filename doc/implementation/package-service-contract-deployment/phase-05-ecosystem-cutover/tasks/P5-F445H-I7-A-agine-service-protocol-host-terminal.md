# P5-F445H-I7-A Agine service, protocol and Host terminal

状态：`READY_FOR_IMPLEMENTATION`。

本节点是 I7 DAG 的 A leaf：完成 Agine service、protocol 与 Host 的 nonlive
current-contract terminal cutover和精确 receipt。直接父节点为：

- `P5-F445H-I7R-cross-boundary-readiness-preflight-result.md`；
- `P5-F445H-I7-S0-real-source-artifact-checkpoint-result.md`；
- `P5-F445H-I7-P0R2-official-packages-prepared-consumer-regate-result.md`；
- `P5-F445H-I7-T0-internals-isolated-gate-tooling-result.md`。

`S0_COMPLETE`、`P0_COMPLETE` 与 `T0_COMPLETE` 均已满足。A 完成后解除 U 与最终 J 的 A
前置；不替代 C、U 或 J 自己的验证。

## 1. Frozen inputs and ownership

| 项 | 值 |
| --- | --- |
| Internals baseline | `1b28fea6925209d668034707a6a57cb72e3c4707` / `1a906f87c663439c022b9ee4f1ad19ed3471f6f1` |
| Skiff source/toolchain | `35133f56eea92b14e40bba6895b23ffd8fdbd9b5` / `febc82d43ed9f623118864f1902a85978de76c0c` |
| official packages | `b06d7aaf16b6914837de1f74920fd3f626040472` / `fb9db28a7d1bd3babafd1dfa7a23687e393ff856` |
| terminal draft stash | `91f3cc32e9d6ce0b14b4145d3d94815ab1a52420` |
| stash provenance base | `19d41001f048efc0b70e13c21d105a855ddd86e2` |
| implementation branch | `codex/p5-f445h-i7-a-agine` |
| implementation worktree | `/Users/geek/workspace/internals-p5-f445h-i7-a-agine` |
| Internals integration owner | `/root/phase05_internals_integration_steward` |
| Skiff task/result owner | `/root/phase05_integration_steward` |

当前 Skiff integration 在 frozen source之后只新增 P0R2 result文档；A 使用的
production/toolchain仍由上述 frozen source定义。

terminal draft必须在新的 clean worktree中从 stash object精确 materialize，不得 `pop`、drop、
rewrite或修改原 stash。T0 相对 stash provenance只修改七个 shared scripts，与 A 写集不重叠。
production首改前，A owner必须记录本 task commit/tree。

## 2. Required terminal behavior

### 2.1 Agine service authoring

- service manifests完成 current split authoring；
- `http.yml` 精确拥有 `43` 个 HTTP entries；
- `websocket.yml` 只声明 connect；
- `service.yml` 只保留 current service identity；
- 不恢复 WebSocket receive/public business method；
- stash中的 `79` 个 `agine/service` 文件可作为机械 caller/fixture syntax migration closure处理。

### 2.2 Host request surface

以下 Host operations必须通过 current three-argument call：

```text
requestJsonToConnection(connectionId, method, value)
```

精确 methods：

```text
host.files.list
host.files.search
host.current-directory
```

transport correlation id由平台隐藏。business params/results必须拒绝 `id`、`requestId` 等
transport字段；普通 WebSocket send仍保持 non-suspending。

### 2.3 Removed legacy paths

terminal source与直接相关测试必须消除：

- database relay/receive路径；
- public cancel surface；
- four-argument connection request；
- deployment timeout作为 service dependency/callee timeout的复用；
- 为旧协议保留的额外兼容分支。

这属于已冻结 current contract 的终态闭合，不是 public-contract redesign。若 private/mechanical
closure不足并要求新的 public method、公开 peer cancellation或 timeout来源，必须停止并返回
`TASK_SCOPE_EXPANDED`。

### 2.4 Host receipt

Host fake必须真实执行上述三个 methods，并证明：

- exact method与business payload到达；
- transport id没有泄漏进business DTO；
- current caller deadline与外层 `timeout(...)` 语义保持；
- current error mapping与terminal failure语义保持；
- 没有回退到 DB relay、receive或public cancel。

## 3. Allowed and forbidden writes

允许：

- Agine service manifests、source与tests；
- 为闭合 current boundary直接因果所需的 Agine protocol与Host files/tests；
- stash中属于上述边界的机械 caller/fixture syntax migration。

禁止：

- `agine/client`；
- Codex Relay、AIHub；
- Internals shared scripts；
- Skiff或official packages；
- stable/watch/reload、live、external network、shared Mongo `27017`、OAuth或browser；
- 与本节点无直接因果关系的public API、权限或用户行为变化。

Internals branch只包含 implementation/tests，不得提交 Phase 05 task/result文档。

## 4. Verification owner

A 是以下 evidence 的唯一 owner：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/check-isolated-service-graph.mjs agine.ai/agine

SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/test-isolated-service.mjs agine.ai/agine
```

同时运行：

- Agine protocol type-check与focused tests；
- Agine Host type-check与focused fake tests；
- 只覆盖改动相关文件的 Node receipt/architecture checks；
- `git diff --check` 与冻结legacy反搜。

本节点不运行 full J join、stable/live/browser/OAuth或外部network gate。runner-owned isolated
temporary runtime/Mongo若由冻结T0 wrapper内建管理，必须与 shared `27017` 隔离并在命令结束时
清理。

## 5. Completion and handoff

完成后 implementation owner把精确 commit/tree、实际写集、focused命令ledger、positive/negative
receipt、cleanup与残余风险结构化回交：

- Internals实现交 `/root/phase05_internals_integration_steward`；
- A result由 `/root/phase05_integration_steward` 落入Skiff Phase 05 task层。

不得自行 merge、清理一级 worktree、rebase或push。若改动需要触碰 C、client、shared scripts、
Skiff/packages、public contract或外部权限，停止并报告 scope expansion。

本证据会因 Agine service/protocol/Host current contract、T0 tooling、Skiff
service-call/current-scope、official package candidate或任一 frozen identity变化而失效。
