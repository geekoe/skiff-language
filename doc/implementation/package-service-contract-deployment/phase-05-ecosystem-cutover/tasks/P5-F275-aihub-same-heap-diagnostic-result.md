# P5-F275 AIHub same-heap diagnostic result

状态：Confirmed diagnostic。

## 父节点与权威链

- consumer 父任务：
  `P5-F269-internals-test-service-migration.md`
- analyzer 父结果：
  `P5-F271-container-projection-heap-cycle-precision-result.md`
- 唯一架构事实源：
  `doc/architecture/package-service-contract-deployment.md` 第 8 节

权威设计要求 compiler 对 boundary-visible effect 做 sound may-analysis：依赖 same-heap identity
的 callable 必须拒绝，但已经被精确 detached 操作消除、不会影响边界可观察行为的局部 identity
不能无条件污染整个 operation。

## Fresh canonical 事实

在 Skiff integration `f69fb5c`（已包含 F271/F273）和新建 canonical artifact store 上逐级
发布后：

- AIHub 共 8 个 intended service operation；
- 6 个为 `Available`，2 个保持 Package-only；
- 两个不可用 operation 的唯一结构化原因都是 `requiresSameHeapIdentity`；
- Agine 因 `managedLlm.streamChat` 不在 fresh ServiceContract 中，在
  `internal.agent_bridge_llm_adapter:109:18` 报
  `for iterable must be Array, Stream, or Map`；
- 因此这不是 Agine iterable 规则错误，也不是旧 artifact 消费。

污染链为：

```text
stream/validateManagedChat
  -> encodeResponsesBody
  -> managedResponsesBodyJson
  -> applyProviderOptions(fresh body, input.providerOptions, provider)
  -> options.get(provider)
```

`Map.get` 取出的 bucket 只进入精确 Fresh、detached 的 `std.json.merge`。该值没有：

- 作为 operation 返回值；
- 逃逸到 stream、callback、spawn、DB、native/external lane；
- 写回 caller-reachable 图；
- 参与 identity 比较或让调用方可观察 alias。

AIHub 源码不应通过复制、编解码或改签名绕过这一分析缺口。

## 当前实现缺口

`compiler/source/src/callable_effects/transfer/call.rs` 在 replay callee 时，只要 callee 带
`requiresSameHeapIdentity` 且相关 actual 是 direct caller reference，就立即把 aggregate
effect 上浮到 caller。该传播发生在 return provenance 被后续精确 Fresh consumer 消除之前，
没有记录 identity requirement 通过哪个返回 projection 才会变得可观察。

F271 已区分 `returnOrigins` 与 `directReturnOrigins`，但 same-heap requirement 仍是独立的
aggregate bool/parameter set；二者尚未形成足够精确的上下文关系。

## 后续边界

下一节点必须先只读确认通用模型，不能只给 `Map.get`、AIHub 或 `std.json.merge` 加名字特判。
至少区分：

- 返回 caller element 后直接返回/逃逸/写入的真实 same-heap 依赖；
- caller receiver 上的独立 identity 比较或写入；
- alias 只进入精确 detached/Fresh producer 的可消除路径；
- conditional、unknown target、跨 Package summary 和 SCC 中无法证明 detached 的失败关闭路径。

当前事实没有暴露新的公共语义选择；它属于权威设计既定 sound analysis 的实现精度问题。若审计
发现必须改变 Package boundary effect wire 或公开 ABI，才升级用户决策。

证据在 analyzer、AIHub production source、PackageArtifact effect summary、canonical std
semantics 或发布顺序变化后失效。

