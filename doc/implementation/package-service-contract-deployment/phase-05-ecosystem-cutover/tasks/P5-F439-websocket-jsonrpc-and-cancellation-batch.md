# P5-F439 WebSocket JSON-RPC与取消语义批次

状态：Superseded by `P5-F440-external-manifest-and-bidirectional-websocket-batch.md`。

F439A与F439C审计结果继续作为后继输入；F439B在用户把WebSocket从outbound-only改为双向declared
JSON-RPC request后停止，未产生result。本文保留为历史任务检查点，不再据此启动新实现。

## 直接父节点与权威设计

- `P5-F438-websocket-outbound-request-response-batch.md`
- `P5-F438A-skiff-websocket-request-owner-audit-result.md`
- `P5-F438B-agine-host-websocket-request-owner-audit-result.md`
- 最终架构事实源：`doc/architecture/package-service-contract-deployment.md`

F438A证明旧任务缺少公开错误投影，不能安全执行。用户随后冻结了替代设计；Skiff commit
`aacee2129934a6aebc2975293b5b4ed4b209c42f`是本批次唯一语义检查点。F438的自定义
`type/requestId/ok` wire和可捕获cancel结论已失效；其关于无用户`receive`、外部上行走HTTP、精确
connection与Host同步读取分类继续有效。

## 已冻结语义

- WebSocket是通用双向transport，不是JSON-RPC专用transport。
- 平台request/response broker拥有编码无关的request identity、pending、connection/generation归属、
  deadline/cancel和容量限制；编码配置只拥有framing与控制字段。
- 第一版只内置`jsonrpc-2.0-text`：一条text frame对应一个JSON-RPC 2.0对象；string `id`由平台生成；
  不支持batch；`params`必须是object或array。
- 取消使用best-effort `$/cancelRequest` notification。Ancestor cancellation是不可捕获的生命周期控制；
  deadline仍抛`TimeoutError`。
- `WebSocketRequestError`是封闭名义union，分为`connectionUnavailable`、
  `transportUnavailable`、`protocolError`、`resourceLimit`和`remote`；不引入`platform.*`字符串错误码。
- JSON编码、非法params shape和success result typed decode使用`std.json.DecodeError`。
- Pending/payload达到上限时新请求fail closed；settled tombstone饱和时驱逐最旧项；平台不自动retry。
- Raw `sendText*`/`sendBinary*`保持原语义。未来binary RPC必须另有显式版本、framing、codec与协商。
- 该能力不增加`service.yml`、`api.yml`或ServiceContract operation。

## 精确输入

| Repo | Integration root | Commit | Tree |
| --- | --- | --- | --- |
| Skiff | `/Users/geek/workspace/skiff-phase-05-integration` | `aacee2129934a6aebc2975293b5b4ed4b209c42f` | `617021923ad3d7072d19deecb9f41460dd2163e4` |
| Internals | `/Users/geek/workspace/internals-phase-05-integration` | `faa11b188c570ca763f107ddd829d52b8fe8861f` | `140d3a03851b64d513fd97c5860e713b8fc314de` |
| skiff-packages | `/Users/geek/workspace/skiff-packages-phase-05-integration` | `f8c634ce4573506e35f6bc1c7cc1e4eef9992a78` | `eb00877ef260d122552af1ff0491c74102adbd57` |

三个integration worktree在本批次开始时clean。禁止访问stable/live，禁止push。

## 当前事实与遮挡

- `std/websocket.skiff`当前只有四个raw send native和两个JSON send helper，没有request API或错误类型。
- Router仍在任意peer data frame上关闭连接，没有broker或JSON-RPC配置adapter。
- Runtime transport只有`connection.send`，没有可挂起的connection request/response frame。
- 生产代码仍把`CancelError`注册为prelude/platform error，并在runtime/router多个层次物化；新设计要求
  区分内部取消终止标记与用户可捕获名义错误，不能只从文档列表删除名字。
- F438B已闭合Host业务分类和大部分consumer owner，但它冻结的是旧自定义wire、string error code和cancel
  envelope；必须做一次窄协议差量审计，不能直接把旧result当实现合同。
- Skiff shared API/wire checkpoint未落地前，Internals consumer实现保持blocked。

## DAG

第一波只读探查：

```text
F439A  CancelError公开面、内部终止与跨边界传播owner审计
F439B  Skiff编码无关broker + JSON-RPC text配置owner审计
F439C  Agine/Host相对F438B的JSON-RPC协议差量审计
```

审计result合流后创建实现leaf，预期结构：

```text
F439A result
  -> cancellation public-surface/internal-terminal shared checkpoint

F439B result + cancellation checkpoint
  -> std/compiler/native + internal transport schema checkpoint
      ├─ runtime suspension/typed codec/cancel leaf
      └─ router broker + jsonrpc-2.0-text adapter leaf
          -> Skiff combined protocol/runtime/router probe

F439C result + Skiff combined checkpoint
  -> Agine service Host caller migration
  -> Host JSON-RPC request/error/cancel peer
  -> legacy relay/receive deletion
  -> Internals combined probe
```

审计若发现共享owner、公共契约或写集与该结构不符，应如实重排；不得要求审计Agent顺手实现。

## 本批次完成标准

- 取消不再是用户可name/catch/throw的`CancelError`，但runtime内部仍能可靠终止和清理work item。
- std、compiler、runtime transport、Router broker与JSON-RPC adapter各有单一owner和可执行任务边界。
- Router核心pending状态不依赖JSON字段；profile adapter不能理解业务payload。
- 正例、乱序、remote error、超时、取消、断线、wrong generation/id、batch、payload/pending/tombstone
  上限均有明确最小证据owner。
- Agine/Host业务代码不感知transport `id`，同步Host读取像HTTP调用一样返回typed result或名义错误；
  durable tool IDs不被删除。
- Skiff与Internals聚焦combined通过后才建立预验收候选；最终gate仍由后续唯一owner执行。
