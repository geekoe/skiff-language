# P5-F440R1 Router RPC core responsibility audit result

状态：`PASS / ATOMIC_KERNEL_AND_STATELESS_SPLITS_PROVEN`。

本节点只读检查F440R后的Router RPC core。没有修改文件、运行测试或启动服务。结论：

- Broker active/tombstone/timer/terminal生命周期必须由一个owner集中管理，不能按inbound/outbound拆；
- JSON-RPC profile可安全拆为public contracts与单一implementation；
- Broker只可抽离无状态wire转换，不能移动任何map/timer/tombstone/writer mutation。

## 1. 必须集中的Broker lease kernel

以下状态和原子转换继续由`WebSocketRequestBroker`单一owner持有：

- generation identity/table；
- outbound peer/runtime indexes；
- inbound peer index；
- 两组tombstone；
- generation active counters与active timer count；
- `settleOutbound -> detachOutbound`；
- `finishInbound -> detachInbound`；
- pre-dispatch error、generation close、runtime disconnect；
- active-token checks、deadline arm/clear。

顺序必须保持：

```text
delete active indexes
  -> decrement counters
  -> clear timer
  -> write tombstone
  -> external cancel/respond/write/abort
```

generation close必须先同时摘除inbound/outbound，再做任何外部effect。因此禁止建立彼此独立的
`InboundBroker`/`OutboundBroker`，否则close/disconnect会跨owner破坏single-terminal原子性。

`webSocketRequestBrokerState.ts`中的有界FIFO/TTL tombstone store保持leaf；active maps不得搬一半进去。

## 2. Profile稳定拆分

建议新建：

```text
protocol/jsonRpc20TextProfileContracts.ts
protocol/jsonRpc20TextProfileImplementation.ts
```

contracts拥有public types/default limit contract；implementation拥有唯一
`JsonRpc20TextProfile` class及所有wire/parser/encoder helper，并依赖contracts与现有
`losslessJson.ts`。

原`jsonRpc20TextProfile.ts`只做facade/re-export：

```text
jsonRpc20TextProfile.ts
  -> jsonRpc20TextProfileImplementation.ts
  -> jsonRpc20TextProfileContracts.ts + losslessJson.ts
```

不得wrapper/subclass，也不得重建default-limits对象；class与constant identity必须保持。

typed-id canonicalization、opaque payload provenance、platform terminal/limit fit仍集中在同一个wire
implementation中，不能分散复制。

## 3. Broker可抽离的唯一边界

可选新建：

```text
router/webSocketRequestBrokerWire.ts
```

只拥有纯转换：

- runtime request -> peer wire request准备；
- peer terminal -> `BrokerRuntimeResponse`；
- inbound dispatch terminal -> peer frame/fallback；
- dispatch result -> inbound terminal。

该模块不得import Broker class/state，不得读写map、timer、tombstone、writer或runtime source。依赖方向只能
是：

```text
webSocketRequestBroker.ts
  -> webSocketRequestBrokerWire.ts
  -> broker types + profile contracts
```

若某段转换需要在external effect前后修改lease状态，则必须留在Broker。

## 4. 验证要求

保持F440R全部60个profile/broker cases，并额外证明：

- external terminal callback中re-enter `debugSnapshot()`时，active/timer/lease已归零且tombstone已存在；
- 从`src/index.ts`和原公开module导入的class/default-limits为同一对象；
- lossless opaque value、typed id、exact encoding、1009 limit/terminal fit不变；
- cancel/complete、duplicate、disconnect、FIFO eviction、late response原子顺序不变。

本审计没有发现功能bug，也没有授权修改broker语义。若结构拆分需要跨模块共享mutable lease state，应保留
现状而不是为了行数继续拆。
