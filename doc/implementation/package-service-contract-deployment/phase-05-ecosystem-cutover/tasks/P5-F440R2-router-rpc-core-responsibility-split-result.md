# P5-F440R2 Router RPC core responsibility split result

状态：`PASS / BEHAVIOR_PRESERVING_RESPONSIBILITY_SPLIT`。

Router RPC core 已完成纯结构拆分：

- `jsonRpc20TextProfile.ts` 成为只 re-export 的 identity-preserving facade；
- public contracts/default limits 与唯一 class/wire implementation 分离；
- Broker 只把无状态 wire/result 转换交给新 leaf；
- generation、active indexes/counters、timer、tombstone、terminal lease 以及全部
  detach/settle/finish/external-effect 顺序仍由 `WebSocketRequestBroker` 单一 owner 集中管理。

没有接入 gateway、`RuntimeEndpoint` 或 `RuntimeDispatcher`，没有改变 public export name、wire schema、
lossless JSON、typed id、terminal outcome 或任何 Broker 状态机行为。

## 1. 基线与提交

| 状态 | Commit | Tree |
| --- | --- | --- |
| 任务声明的 implementation baseline | `78fc2abc2b76a671bf5ebbd42d58011bc1be804d` | `5d26580a35e3c83f01e07d7c841a14e48e4bff5c` |
| worktree 实际起点 | `283e2c27b7510ab992cae7537105f7e5daeade36` | `ee0d887198db8af3bb5245c7cb56a2b0e30e07c1` |
| implementation | `580548dbdb75080bdbe920407f4ba54c0f006cad` | `38a17d75a6cb571cd2cb61d50eeceec2a35cbe5a` |

`78fc2abc..283e2c27` 只新增本 leaf task 文件，没有 production/test 差异。Implementation 与本文 result
分离提交；result commit/tree 由最终交付消息记录。

## 2. Test-first red 与原子顺序回归

production 拆分前先加入 facade identity 测试，并从计划中的 contracts/implementation 路径直接导入。
第一次聚焦执行得到真实 compile-red：

```text
1 failed suite / 0 tests
Cannot find module '../src/protocol/jsonRpc20TextProfileContracts.js'
```

随后加入两个 reentrant terminal 用例；它们在移动 production 代码前已对现有 Broker 顺序通过，并继续锁定：

- outbound runtime source 的 `respond()` 内重入 `debugSnapshot()` 时，两个 outbound index、
  generation active、timer、terminal lease 均已从 `1` 归零，outbound tombstone 已为 `1`；
- inbound peer writer 的 `writeText()` 内重入时，inbound index、generation active、timer、
  terminal lease 均已从 `1` 归零，inbound tombstone 已为 `1`。

因此 terminal callback 不可能观察到半摘除 lease。

## 3. Profile responsibility split

`jsonRpc20TextProfileContracts.ts` 现在唯一拥有：

- profile/id/opaque payload/public action 与 response types；
- limits contract 与唯一 frozen `DEFAULT_JSON_RPC_20_TEXT_LIMITS` 对象；
- outbound id generation、platform error 和 adapter interfaces。

`jsonRpc20TextProfileImplementation.ts` 现在唯一拥有原 `JsonRpc20TextProfile` class 以及全部 parser、
encoder、typed-id、opaque payload、terminal-fit 和 limit helper；实现只依赖 contracts 与
`losslessJson.ts`。

原 `jsonRpc20TextProfile.ts` 只有两条 re-export。`router/src/index.ts` 未修改。回归测试以
`toBe` 证明从 index、原 facade、contracts/implementation 导入的 class/default limits 保持 strict
identity；没有 wrapper、subclass、第二个 class 或第二个 default object。

## 4. Broker responsibility split

`webSocketRequestBrokerWire.ts` 只接受显式输入并返回转换结果，拥有：

- runtime opaque params materialization 与 outbound peer request frame encoding；
- peer response terminal 到 `BrokerRuntimeResponse` 的映射；
- inbound dispatch result 到 terminal/abort plan 的映射；
- inbound terminal encoding/fallback 与 best-effort cancel frame 选择。

该 leaf 不 import Broker class/state，不接受 map、writer、runtime source、`AbortController` 或 external
callback。

以下 mutable lease kernel 全部仍在 `WebSocketRequestBroker`：

- generation table/identity/uid、outbound id sequence 与 runtime sender identity；
- outbound peer/runtime indexes、inbound peer index；
- 两组 tombstone store；
- generation/global active counters、timer count 与 deadline scheduling；
- active-token checks、`settleOutbound`、`finishInbound`、两组 detach、generation close；
- writer/source/abort/protocol-violation external effects。

原子顺序未移动：active indexes/counters/timer 先清理并写 tombstone，之后才 cancel/respond/write/abort。
generation close 仍先统一 detach inbound/outbound，再执行任何 external effect。

## 5. 聚焦验证

任务给出的 pnpm wrapper 行为与 F440R 记录一致：

- `pnpm --dir router exec vitest list --root router ...` 退出 `0` 但无 listing；
- `pnpm --dir router exec vitest run --root router ...` 把 root 解析为 `router/router`，以
  `No test files found` 退出 `1`。

因此按任务允许的 fallback 直接调用现有 Vitest binary。依赖只通过命令生命周期内的临时 symlink
提供，退出时全部删除。

| Check | Result |
| --- | --- |
| direct Vitest listing | PASS，`63` non-zero：profile `35`、broker `28` |
| direct Vitest execution | PASS，`2 files / 63 tests` |
| `pnpm --dir router type-check` | PASS |
| implementation `git diff --cached --check` | PASS |

63 cases 包含 F440R 原 60 cases，以及 1 个 facade identity 和 2 个 reentrant terminal regression。
原有 lossless opaque request/response、typed id、exact encoding、1009 limit、cancel-vs-complete、
duplicate、disconnect、tombstone FIFO/TTL/late-terminal cases 全部继续通过。

额外使用只读 Node import-graph scan 检查相关可达模块：

```text
facade -> contracts + implementation
implementation -> contracts + losslessJson
contracts -> leaf
Broker -> contracts + state/types + wire
wire -> contracts + broker types
```

结果为 `8 reachable modules / 0 cycles / 0 forbidden wire dependencies`。

## 6. Scope audit

Implementation 只修改/新增任务允许的 7 个文件：

- profile facade/contracts/implementation；
- Broker owner 与新 wire leaf；
- 两个 direct test 文件。

`router/src/index.ts` public surface 保持原样；没有修改 Broker state/types leaf、gateway/server、
`RuntimeEndpoint`、`RuntimeDispatcher`、wire schema、Rust、fixture、README 或其它 task/result。

只运行了指定的聚焦 Router tests、type-check 与静态扫描；未运行完整 Router suite，未启动 server、
instance、stable、live 或 network；未派子 Agent，未 merge、rebase 或 push。
