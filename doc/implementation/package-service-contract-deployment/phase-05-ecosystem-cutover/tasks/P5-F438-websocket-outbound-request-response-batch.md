# P5-F438 WebSocket outbound request/response batch

状态：Ready for bounded audits。权威设计已更新；当前是实现检查点，不是稳定候选。

## 直接父节点与权威设计

- `P5-F436B-agine-request-id-terminal-owner-audit-result.md`
- `P5-F437A-aihub-canonical-publish-path-closure-audit-result.md`
- `doc/architecture/gateway-runtime-adapter-boundary.md`
- 最终架构事实源：`doc/architecture/package-service-contract-deployment.md`

2026-07-27用户澄清了F436B未冻结的Host方向。Skiff仍只通过HTTP接收外部peer主动发起的业务请求；
但Skiff可以通过WebSocket向Host等外部peer发起请求，并在同一连接上等待平台response。该response不是
Host向Skiff发起的新请求，不创建service ingress，也不改变`service.yml`。

权威设计已在Skiff `64a0ab4ec85d25899dc8563ac6d647edad8ed23e`冻结：

- `service.yml.websocket`仍只有path与可选connect，不增加receive/message/request route；
- 普通`connection.send`仍是non-suspending单向通知；
- `std.websocket.requestJsonToConnection<TRequest, TResponse>`向精确connection发起请求并挂起等待；
- 固定text JSON envelope使用平台生成的transport `requestId`；
- Router专用broker精确按connection/socket generation/request id关联response，不调用用户handler；
- request与success response分别按`std.json.encode<TRequest>` /
  `std.json.decode<TResponse>`处理；
- cancel/deadline/disconnect清理pending并使用有界settled tombstone处理晚到竞态；
- client-initiated request/notification仍拒绝，业务上行仍走HTTP。

本批次显式取代F424“所有client data frame一律1003”的过宽实现结论，但不改写历史任务/result。
F424关于connect-only authoring、无用户receive和HTTP业务上行的结论继续有效。

## 精确输入

| Repo | Integration root | Commit | Tree |
| --- | --- | --- | --- |
| Skiff | `/Users/geek/workspace/skiff-phase-05-integration` | `64a0ab4ec85d25899dc8563ac6d647edad8ed23e` | `562adcfc8baa595969a4dd1ccd2e67c4053814b9` |
| Internals | `/Users/geek/workspace/internals-phase-05-integration` | `066b5135a8e06f87acfd614e408e05b35453f4eb` | `23be114f0d4b838eff1c7b214a40fc9c57cdd354` |
| skiff-packages | `/Users/geek/workspace/skiff-packages-phase-05-integration` | `f8c634ce4573506e35f6bc1c7cc1e4eef9992a78` | `eb00877ef260d122552af1ff0491c74102adbd57` |

三个integration worktree建立本批次时均clean。Skiff设计commit是后续共享wire/API checkpoint的唯一
语义输入；实现任务不能自行增加client-initiated handler、business-identity unary fan-out或兼容旧envelope。

## 当前事实与遮挡

- Router current gateway在任意client data frame上直接以`1003`关闭；没有peer response broker。
- Runtime current `connection.send`是单向control frame；没有可恢复等待的connection request/response
  transport。
- `std.websocket`只导出四个native send与两个JSON helper；native registry/effect/required-context尚无
  request callable。
- Runtime已经有service call、HTTP和stream suspension/cancel owner，但不能假定WebSocket request可直接
  复用任意pending map；必须先审计确切生命周期和frame owner。
- Agine保存Host的精确active connection id，现有Host file/tool链却自行持久化relay request id并依赖
  legacy service receive dispatcher；新设计允许把有限同步Host操作收敛为平台请求。
- F437A的canonical workflow root遗漏与本批次wire实现互不依赖，可并行修复；Agine publish与最终assembly
  仍被legacy WebSocket authoring、config binding及HTTP selector collision遮挡。

## DAG

第一波：

```text
F438A  Skiff std/native/runtime/router真实owner只读审计
F438B  Agine/Host请求、通知与HTTP上行真实owner只读审计

F437B  canonical three-repo root/provenance workflow repair（独立并行）
```

审计result合流后才创建实现leaf：

```text
F438A result
  -> shared std/native + internal wire/schema checkpoint
      ├─ runtime suspending request/cancel/typed codec
      └─ router exact-connection broker/generation/tombstone
          -> Skiff combined protocol/runtime/router probe

F438B result + Skiff combined checkpoint
  -> Agine service Host caller migration
  -> Host peer request/cancel/response implementation
  -> legacy receive/request relay deletion
  -> Internals focused combined

F437B + Agine config/authoring + selector owner decision/repair
  -> cheap full canonical publish/assembly probe
  -> stable candidate evidence coverage
```

共享schema/wire/API必须先落单一检查点。Runtime和Router只能在该检查点之后按互斥写集扇出。
如果审计发现正确实现还需要新的公共语义、额外service.yml authoring或无法确定的Host业务生命周期，
先返回scope expansion，不让开发Agent自行决定。

## 批次完成标准

- 固定平台envelope、std API、内部runtime wire与effect/suspension metadata有单一代码owner。
- Router能安全关联乱序response，且wrong connection/generation/id、cancel竞态、容量与断线均fail closed。
- Runtime只恢复原execution，typed JSON错误与transport错误保持不同名义类型。
- Agine/Host同步请求不再自建transport requestId/pending relay；真正durable业务ID继续保留。
- 外部peer主动上行全部走HTTP；`service.yml`仍无receive/message route。
- 聚焦Skiff与Internals combined通过后，canonical publish/assembly路径可继续收敛。
- 未经用户明确授权不push，不访问stable/live；最终阶段收尾再合入`main`并清理worktree。
