# P5-F445H-I7-C Codex Relay and AIHub current contract

状态：`READY_FOR_IMPLEMENTATION`。

本节点是 I7 DAG 的 C leaf：让 Codex Relay provider与唯一真实 caller AIHub完成 current
split-authoring、current inline-effect和nonlive exact receipt。直接父节点为：

- `P5-F445H-I7R-cross-boundary-readiness-preflight-result.md`；
- `P5-F445H-I7-S0-real-source-artifact-checkpoint-result.md`；
- `P5-F445H-I7-T0-internals-isolated-gate-tooling-result.md`；
- final I6 acceptance。

`P5-F445H-I7-P0R2-official-packages-prepared-consumer-regate-result.md` 是并行阶段事实，但 C 不依赖
P0R2。C 完成后解除 U 与最终 J 的 C 前置；C 不依赖、不修改也不替代 A。

## 1. Frozen inputs and owners

| 项 | 值 |
| --- | --- |
| Internals baseline | `1b28fea6925209d668034707a6a57cb72e3c4707` / `1a906f87c663439c022b9ee4f1ad19ed3471f6f1` |
| Skiff code/toolchain source | `35133f56eea92b14e40bba6895b23ffd8fdbd9b5` / `febc82d43ed9f623118864f1902a85978de76c0c` |
| official packages | `b06d7aaf16b6914837de1f74920fd3f626040472` / `fb9db28a7d1bd3babafd1dfa7a23687e393ff856` |
| implementation branch | `codex/p5-f445h-i7-c-relay-aihub` |
| implementation worktree | `/Users/geek/workspace/internals-p5-f445h-i7-c-relay-aihub` |
| Internals integration owner | `/root/phase05_internals_integration_steward` |
| Skiff task/result owner | `/root/phase05_integration_steward` |

当前 Skiff integration 在 frozen code/toolchain source之后只有 P0R2与本阶段 task docs增量；
C使用的 production、scripts、tests、Cargo与lockfile输入仍由上述 frozen source定义。
production首改前，C owner必须返回本 task commit/tree。

## 2. Read-only preflight facts

baseline 的 Relay/AIHub `package.yml`、`http.yml`、`service.yml` 已完成 split authoring；
`service.yml` 只包含 identity/service calls。因此 C 默认不得重复修改manifest。

当前具体缺口：

- 两个 service root仍有 legacy `skiff.test-doubles.json`，current runner明确拒绝；
- AIHub已有静态 caller，但没有 exact current service-effect执行 receipt；
- Relay API-key raw success stream没有
  `start(status, filtered headers) -> ordered surviving chunks/events -> single end` receipt；
- Codex sidecar另有两个 per-test admin configs；F267 已删除 per-test config。

机械闭合策略：

- 各 service新增普通 `config.skiff-test.yml`；
- sidecar effects迁入其所属 `.test.skiff` inline effects；
- 两个 remote admin用例只在test文件内调用既有 private
  `adminHttpConfig`/boundary/session primitives并传显式 settings；
- OAuth request double只迁入 test inline effect；
- 不修改 admin/OAuth production。

## 3. Required current behavior

### 3.1 Relay provider

Codex Relay必须保留 tagged unary export与current service identity。允许在
`service/proxy_runtime.skiff` 抽取一个 private raw-response forwarding seam，但：

- `std.http.stream` 仍是唯一 production upstream入口；
- public API、status/header/body/event语义和权限不变；
- API-key external success必须按顺序发出：
  `start(status, filtered headers)`、所有 surviving ordered chunks/events、恰好一次 `end`；
- sensitive headers继续过滤；
- malformed SSE、split UTF-8、JSON/SSE sanitize与既有有限 failure语义继续保留。

不得声称所有headers或raw chunk boundaries bit-for-bit保留；current contract只保证过滤后的
headers与 surviving events/chunks的相对顺序。

### 3.2 AIHub bounded follower

AIHub必须以内联 service effect命中精确 operation：

```text
codexRelay/relayProxy.responsesCompletedResult
```

receipt必须执行该 effect，并证明 completed output按current contract投影。request中不得出现
Router selectors或transport correlation字段。

provider implementation commit必须先于 AIHub caller commit；caller不得通过复制 provider、
mock dot-service identity或绕过 exact service operation闭合。

### 3.3 Negative closure

改动后必须反搜/证明：

- legacy test sidecar为零；
- old dot-service relay为零；
- public cancel为零；
- deployment timeout作为dependency timeout复用为零；
- malformed SSE与sensitive-header过滤仍保持；
- failure-before-start / failure-after-start使用既有 finite prestart/poststart matrices及Relay
  scoped transport证据，不发明 public cancel。

## 4. Allowed write set

Codex Relay允许：

```text
codex-relay/service/proxy_runtime.skiff
codex-relay/service/relay_routes.test.skiff
codex-relay/service/relay_responses_projection.test.skiff
codex-relay/service/package_response_health.test.skiff
codex-relay/service/admin_http.test.skiff
codex-relay/service/chatgpt_oauth.test.skiff
codex-relay/service/config.skiff-test.yml
codex-relay/service/skiff.test-doubles.json
```

其中：

- `proxy_runtime.skiff` 只允许 private raw-response forwarding seam；
- projection/health tests只在 focused receipt证明必要时修改；
- admin/OAuth tests只允许上述 mechanical fixture migration；
- 新增普通 config，并删除 legacy sidecar。

AIHub允许：

```text
aihub/service/internal/aihub_service.test.skiff
aihub/service/config.skiff-test.yml
aihub/service/skiff.test-doubles.json
```

`aihub_service.skiff` 只有 exact compiler RED证明需要 mechanical caller seam时才允许修改。
`service.yml`、`package.yml`、`http.yml`、`api.yml` 只有 exact compiler RED证明current authoring
closure缺失时才允许机械修改；否则必须零diff。

## 5. Forbidden scope and stop conditions

禁止：

- Agine service/client/Host；
- Internals shared scripts；
- `llm` public API；
- Skiff或official packages；
- admin/OAuth production；
- external login/network、stable/watch/reload、shared Mongo `27017`、browser或live；
- public contract、用户行为、权限或公开 cancellation变化。

若 private seam不足并要求 public contract、admin/OAuth production、shared scripts/T0 graph、
A-owned文件或外部权限，立即停止并返回 `TASK_SCOPE_EXPANDED`。机械 caller/fixture closure可在
上述边界内自行完成。

Internals branch只包含 implementation/tests，不得提交 Phase 05 task/result文档。

## 6. Verification owner

C 是下列 isolated graph/matrix 的唯一 owner：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/check-isolated-service-graph.mjs agine.ai/codex-relay

SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/test-isolated-service.mjs agine.ai/codex-relay

SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/check-isolated-service-graph.mjs agine.ai/aihub

SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/test-isolated-service.mjs agine.ai/aihub
```

另运行只覆盖改动相关本地文件的必要 Node receipt/syntax检查和 `git diff --check`。允许冻结
runner管理的isolated temporary Mongo/runtime，但必须与 shared `27017`/stable隔离并在命令结束
清理。

不得运行 build/dev/start、browser/OAuth/network或 full J gate。

## 7. Completion and handoff

先提交 Relay provider implementation，再提交 AIHub caller/receipt。完成后 implementation owner
结构化回交：

- task/Relay/AIHub/final commits与trees；
- 精确实际写集及相对 frozen baseline diff；
- isolated graph/matrix与focused receipt command ledger；
- positive、negative、cleanup与残余风险；
- worktree/branch status。

Internals实现交 `/root/phase05_internals_integration_steward`；完整 C result交
`/root/phase05_integration_steward` 落入 Skiff task层。开发 owner不得自行 merge、清理一级
worktree、rebase或push。

本证据会因 Relay/AIHub contract、LLM stream shape、T0 scripts、Skiff
service-call-current-scope、official package candidate或任一 frozen identity变化而失效。
