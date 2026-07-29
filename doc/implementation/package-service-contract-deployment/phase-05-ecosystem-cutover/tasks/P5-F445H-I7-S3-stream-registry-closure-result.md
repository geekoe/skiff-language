# P5-F445H-I7-S3 stream registry 闭合结果

状态：

```text
PASS
S3_COMPLETE = YES
PRODUCTION_CHANGE = NONE
BLOCKING_ISSUES = 0
```

M5 中 AIHub case 28、29 的 `unknown Stream value` 在 M6 精确复跑中均未再出现。
临时诊断轨迹证明这两个用例的 producer 与 consumer 使用同一个请求级 registry：
所有 stream 都在 lookup 前注册，在终态后释放，scope 关闭时活动 stream 数为零。

本节点没有足够证据证明存在新的 stream registry 缺陷，因此不引入猜测性修复。

## 1. 执行身份

| 项 | 值 |
| --- | --- |
| Skiff baseline commit/tree | `b4bdbddb8761bcf053258eef5b87b778c3299b7a` / `7d81c6ef01cb47c2a7904cdc48ccd8f4d11a9ed7` |
| Internals baseline | `9c3bdc` |
| G2 production candidate commit/tree | `04f50abd5aaf0bcd962129c5824121e87a959545` / `37044f0155d8f8f43323b11811362aacd7a253c4` |
| M6 production integration commit/tree | `51487de4` / `b18fa9e7357d723a7fff2de3ac3401fa769e01ff` |
| branch | `codex/p5-f445h-i7-s3-stream-registry-closure` |
| integration owner | `/root/phase05_integration_steward` |

result commit/tree 由 Git handoff 报告；本文档不自引用自己的 commit。

## 2. 问题复现与定位

M5 的默认 51 用例结果为 `15 PASS / 36 FAIL`：

- 34 个用例失败于 `unsupported native target std.json.encode`；
- case 28、29 失败于 `unknown Stream value`。

case 28、29 的源码链是请求内的四层本地/package producer 转发：

```text
test case
  -> aihub_service.completedResponseSse
  -> llmApi.decode.decode
  -> responsesToLlmStreamEvents
  -> source stream
```

该链没有跨服务 stream ABI，也没有 Router/Host 公共身份传输。按相同四层
producer/deferred/consumer 形状构造的纯 Eval 临时 fixture 在原 baseline 上即通过，因此
不能把 M5 现象归因为普通嵌套 producer 必然丢失 registry。

## 3. M6 精确证据

M6 在 G2 的 production candidate 上复跑相同默认 51 用例，结果为：

```text
47 PASS / 4 FAIL
case 28 = PASS
case 29 = PASS
unknown Stream value = 0
unsupported native target std.json.encode = 0
```

剩余 4 个失败都是另一个已知边界：

```text
std.http.emitResponseStream used outside a raw HTTP streaming response context
```

它们不属于 S3。

临时诊断轨迹中：

| 用例 | runtime/scope | registry 证据 |
| --- | --- | --- |
| case 28 | `28/28` | `stream-0..3` 均先 create、后 lookup；终态逐一 finish；`active=0` 后关闭 scope/owner |
| case 29 | `29/29` | `stream-0..3` 均先 create、后 lookup；终态逐一 finish；`active=0` 后关闭 scope/owner |

两条轨迹都包含 deferred stream 的 insert/take 配对，未出现跨 runtime lookup、缺失登记、
提前关闭或终态残留。

## 4. 判定边界

能够确认：

- 当前 production candidate 上，原 case 28、29 精确通过；
- 同一请求内的四层 stream handoff 使用同一个 registry；
- 终态清理保持 S2 的 `End`、error、cancel 所有权，没有活动 stream 残留；
- G2 只修复动态 `std.json.encode` 返回值物化路径，没有修改 stream registry。

不能确认：

- M5 的 `unknown Stream value` 是否由更早的 encode/exception 失败次生触发；
- 或者它是否属于当前已不可复现的历史执行状态。

因此本结果只记录“上游 encode/exception 失败共现的次生现象或当前不可复现”，不宣称
G2 修复了 stream registry，也不为未知根因编造新的生产修复。

## 5. 写集与清理

最终可集成写集只有本文档。诊断期间临时改过：

```text
runtime/host/src/capability_context/stream_runtime.rs
runtime/eval/src/program_stream.rs
```

这些改动只输出脱敏的 registry owner/handoff 事件，已经完整撤回。最终树相对于 baseline：

```text
runtime production diff = empty
public Stream ABI diff = empty
Router/Host behavior diff = empty
```

诊断 commit `bfec329d1bc55113865074871544d0b098e6702e` 及 M6 临时诊断 commit
`8b174dba` 禁止集成。S3 未使用 MongoDB、网络、stable/live instance。

## 6. 验收

| 条款 | 判定 |
| --- | --- |
| case 28、29 精确复跑 | PASS |
| `unknown Stream value` 归零 | PASS |
| creator/consumer registry owner 一致 | PASS |
| deferred insert/take 配对 | PASS |
| terminal 后 `active=0` | PASS |
| S2 End/error/cancel 清理未改动 | PASS |
| 无猜测性 production fix | PASS |
| trace-only 改动不集成 | PASS |

因此：

```text
S3_COMPLETE = YES
PRODUCTION_CHANGE = NONE
BLOCKING_ISSUES = 0
```

本节点只闭合 M5 的两个 `unknown Stream value` 现象，不宣告整个 I7 完成。
