# P5-F445H-I7-C1 Codex Relay provider checkpoint result

状态：

```text
PARTIAL
C_COMPLETE = NO
C1_PROVIDER_CHECKPOINT = YES
C2_AIHUB_CONTINUATION = RUNNING
```

Codex Relay provider已完成有界current-contract implementation checkpoint：legacy test sidecar已迁移
到current inline effects/test config，API-key raw SSE经private forwarding seam保留filtered
headers、surviving chunk/event顺序与single terminal，focused graph与Node receipt通过。

本节点不是完整C结果。isolated service test虽已越过parse/type-model，但连续在current isolated
assembly prepare的`20s`预算返回`504`，没有进入test assertions。因此不得把C1写成PASS，不解除
U或J的C前置；AIHub caller与combined C证据由C2继续。

## 1. Parent and exact identities

直接task contract：

```text
P5-F445H-I7-C-codex-relay-aihub-current-contract.md
```

| 项 | 值 |
| --- | --- |
| C task commit/tree | `fb2be66920c202b295afdc6c5205eb7be14d42f8` / `0c42d4ab88db65041f439a801aa712116443650c` |
| Internals baseline | `1b28fea6925209d668034707a6a57cb72e3c4707` / `1a906f87c663439c022b9ee4f1ad19ed3471f6f1` |
| C1 candidate commit/tree | `bb425a484ca4e99600338cbfb60da174250137b6` / `0212de6125f7fa6f2099676dae9f135fb739562e` |
| C1 branch | `codex/p5-f445h-i7-c-relay-aihub` |
| C1 worktree | `/Users/geek/workspace/internals-p5-f445h-i7-c-relay-aihub` |
| C2 continuation branch | `codex/p5-f445h-i7-c2-aihub` |
| C2 continuation worktree | `/Users/geek/workspace/internals-p5-f445h-i7-c2-aihub` |

C1交接时worktree/branch clean。AIHub WIP已无损迁移到以
`bb425a484ca4e99600338cbfb60da174250137b6` 为base的C2 worktree/branch。C1 candidate将由
Internals integration steward负责合流和一级清理；本结果不声称它已经进入Internals integration
branch。

## 2. Actual write set

相对Internals baseline精确为Codex Relay service下七个文件：

```text
M codex-relay/service/admin_http.test.skiff
M codex-relay/service/chatgpt_oauth.test.skiff
A codex-relay/service/config.skiff-test.yml
M codex-relay/service/proxy_runtime.skiff
M codex-relay/service/relay_routes.test.skiff
D codex-relay/service/skiff.test-doubles.json
M codex-relay/service/upstream_health.skiff
```

name-status ledger SHA-256：

```text
becbaa668da48f52c57ac8ac07d803ae741caf403b5c921e5b8be3e6d091a1dd
```

没有AIHub、Agine、shared Internals scripts、Skiff、official packages、public API manifest或
lockfile写入。`upstream_health.skiff` 只做current parser所需的mechanical private name
migration，不改变health行为、public contract或external request ownership。

## 3. Delivered provider checkpoint

### Current test authoring

- legacy `skiff.test-doubles.json` 已删除；
- 新增普通 `config.skiff-test.yml`；
- Relay sidecar effects迁入所属 tests的current inline effects；
- admin tests只调用既有private boundary/session primitives并使用显式settings；
- OAuth request double只在test中inline迁移；
- admin/OAuth production保持零diff。

### Raw response forwarding

`proxy_runtime.skiff`只抽取private raw-response forwarding seam，production upstream入口仍是
native `std.http.stream`。archive-off raw SSE receipt证明：

```text
start(status, filtered headers)
  -> ordered surviving SSE chunks/events
  -> exactly one end
```

sensitive header过滤、current source health、event顺序与single terminal保持。没有新增公开cancel、
transport field或public API。

## 4. Evidence ledger and current blocker

| Evidence | 结果 |
| --- | --- |
| Codex Relay isolated graph check | PASS |
| Node service-api receipt | PASS `4/4` |
| `git diff --check` | PASS |
| Codex Relay isolated service test | BLOCKED；已越过parse/type-model，连续在isolated assembly prepare `20s`预算返回`504`，未进入assertions |

因此当前准确判定是：

```text
C1_PROVIDER_CHECKPOINT = YES
C_COMPLETE = NO
```

`504` 不是test assertion FAIL，也不能被focused receipt替代。C2/combined C owner必须在冻结
Internals integration identity上重新获得non-zero isolated service execution，或结构化定位并关闭
assembly prepare budget blocker，才能给出完整C结果。

## 5. Isolation and continuation

C1没有访问stable/live、external network、shared Mongo `27017`、OAuth或browser，没有push。
worktree handoff时clean。任何runner-owned temporary state必须由对应isolated命令owner清理；本节点
不把未进入assertion的run计作runtime PASS。

C2必须继续完成：

- AIHub exact inline service effect：
  `codexRelay/relayProxy.responsesCompletedResult`；
- completed output projection与无Router selector negative；
- Relay provider + AIHub caller在最终Internals integration identity上的isolated graph/matrix；
- C task定义的legacy反搜、cleanup与完整command ledger。

只有combined result满足task全部positive/negative与non-zero isolated evidence后，才能记录：

```text
C_COMPLETE = YES
```

Relay/AIHub contract、LLM stream shape、T0 scripts、Skiff service-call-current-scope、C1/C2 candidate
identity或actual write set变化会使相应checkpoint证据失效。
