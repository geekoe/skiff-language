# P5-F440R2 Router RPC core responsibility split

状态：Ready。F440R的behavior-preserving结构收尾；不接gateway/RuntimeDispatcher。

## 直接父节点

- `P5-F440R1-router-rpc-core-responsibility-audit-result.md`
- `P5-F440R-router-websocket-rpc-profile-broker-core-result.md`

F440R1冻结必须集中的Broker lease kernel及可拆的profile/wire边界；F440R冻结全部profile/broker行为和
60个direct tests。本leaf只移动职责并增加原子顺序回归，不改变public API或状态机。

实现基线为`78fc2abc2b76a671bf5ebbd42d58011bc1be804d`。

## 目标

1. 把JSON-RPC profile的public contracts与唯一wire implementation拆开，原module保持identity-preserving
   facade；
2. 从Broker只抽离不读写mutable state的wire/result转换；
3. Broker继续独占generation、active indexes、tombstone、timer、terminal lease及所有detach/settle顺序；
4. public export names、class/object identity、constructor行为与F440R测试完全不变。

本节点只形成结构检查点；R0b继续从原public facade消费。

## 唯一写集

- `router/src/protocol/jsonRpc20TextProfile.ts`
- 新建：
  - `router/src/protocol/jsonRpc20TextProfileContracts.ts`
  - `router/src/protocol/jsonRpc20TextProfileImplementation.ts`
- `router/src/router/webSocketRequestBroker.ts`
- 新建`router/src/router/webSocketRequestBrokerWire.ts`
- `router/src/index.ts`只允许保持/re-export同一public surface
- `router/tests/json-rpc-20-text-profile.test.ts`
- `router/tests/websocket-request-broker.test.ts`
- 本leaf result

禁止修改broker state/types leaf、lossless JSON语义、RuntimeEndpoint、RuntimeDispatcher、gateway/server、
wire schema、Rust、fixture、README、其它task/result。不得派子Agent，不得启动server/live/network。

## Profile split

contracts模块移动且仅拥有public type/constant contract，包括：

- profile/id/opaque payload类型；
- limits与唯一default limits对象；
- outbound id generation；
- response/platform error/action/adapter interface。

implementation模块拥有同一个`JsonRpc20TextProfile` class和当前全部private parser/encoder/helper。它只依赖
contracts与`losslessJson.ts`。

原`jsonRpc20TextProfile.ts`只re-export，不能：

- wrapper/subclass；
-复制class；
- 重建/freeze第二个default limits对象；
- 复制typed-id、opaque payload、terminal或limit逻辑。

从`src/index.ts`与原module导入必须得到strict-equal的class/default object。

## Broker split

新wire模块只接受显式immutable输入并返回纯值，可拥有：

- outbound peer frame准备；
- peer terminal到runtime response映射；
- inbound dispatch result到terminal/frame映射；
- 无状态fallback frame选择。

以下必须留在`WebSocketRequestBroker`：

- 所有map/tombstone/timer/generation/counter；
- attach/detach/settle/finish；
- external writer/source/abort调用；
- active token校验；
- cancel/deadline/disconnect/close race；
- 在external effect前完成lease归零与tombstone写入的顺序。

wire模块不得import Broker class/state，也不得接受可变map、writer、runtime source或callback。若候选helper无法
满足该条件，保留在Broker。

## Test-first与验证

先新增facade identity与reentrant terminal snapshot断言，使未拆分布局或未显式保证顺序至少一项失败，再
移动代码。必须覆盖：

- F440R原60个tests保持通过；
- index/module class与default-limits strict identity；
- terminal writer/source callback内`debugSnapshot()`观察到对应active/timer/lease为0且tombstone已写；
- lossless opaque request/response、typed id、exact encoding、1009 limit；
- cancel-vs-complete、duplicate、disconnect、tombstone FIFO/late terminal行为不变；
- dependency scan证明facade/implementation/contracts与Broker/wire方向无循环。

必跑：

```bash
pnpm --dir router exec vitest list --root router \
  tests/json-rpc-20-text-profile.test.ts \
  tests/websocket-request-broker.test.ts
pnpm --dir router exec vitest run --root router \
  tests/json-rpc-20-text-profile.test.ts \
  tests/websocket-request-broker.test.ts
pnpm --dir router type-check
git diff --check
```

pnpm wrapper若仍误解root，按F440R result使用现有Vitest binary；先证明非零listing并记录实际count。

## 停止与交付

若保持public identity或terminal原子顺序需要跨模块共享mutable state，保留对应逻辑不拆；若整个拆分前提
被证伪，返回`TASK_NOT_EXECUTABLE`，不得改行为完成行数目标。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440r2-router-rpc-core-split`
- branch：`codex/p5-f440r2-router-rpc-core-split`
- result：`P5-F440R2-router-rpc-core-responsibility-split-result.md`

Implementation与result分开提交；不merge/rebase/push。
