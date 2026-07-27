# P5-F439B Skiff WebSocket broker与JSON-RPC owner审计

状态：Ready。高风险、只读shared transport/runtime owner审计。

## 直接父节点

- `P5-F439-websocket-jsonrpc-and-cancellation-batch.md`

父节点记录已冻结语义、精确输入、当前遮挡和后续DAG。启动时只读本任务；需要依据时沿引用链向上读取。
F438A result只作为旧审计为何停止的证据，不再拥有wire或错误语义。

## 输入与目的

| commit | tree |
| --- | --- |
| `aacee2129934a6aebc2975293b5b4ed4b209c42f` | `617021923ad3d7072d19deecb9f41460dd2163e4` |

本审计一次列清`requestJsonToConnection`从source typing到Router物理socket的production/test owner，并按
“编码无关broker + `jsonrpc-2.0-text` adapter”拆出可并行实现写集。它不实现、不修改fixture、不运行
完整gate。

## 只读范围

允许读取：

- `std/**`
- `compiler/**`中std callable、generic native、effect、suspension与required-context owner
- `runtime/**`中native dispatch、typed codec、capability context、pending、cancel与transport
- `router/**`中runtime protocol/endpoint、WebSocket gateway、connection/generation索引与tests
- `artifact-model/**`、`test-runner/**`、`scripts/**`中直接生成或校验上述surface的owner
- `cross-system-fixtures/package-service-ecosystem/**`中的TS/Rust runtime wire fixture
- 父节点及其引用链

唯一允许写入是本leaf result。禁止修改production/test/design、Internals或skiff-packages。

## 必须回答

1. 画出当前`connection.send`完整链，列definition、producer、consumer、schema/generation常量、fixture和
   聚焦测试；标出raw send必须保持non-suspending的保护点。
2. 找出最接近“native发起后挂起等待外部response”的现有实现，核对continuation、deadline/cancel、
   exactly-once completion、disconnect cleanup、actor executor release和generic concrete type plan；
   不得凭名字假定HTTP/service-call pending可复用。
3. 冻结shared std/compiler/runtime transport checkpoint：
   - `WebSocketRequestError`封闭union与export；
   - generic native signature、type arguments、JSON encode/params shape/result decode；
   - `maySuspend`、effect、target/conflict key、cancel safety与required context；
   - runtime-originated request/cancel和Router-originated response的最小frame；
   - TS/Rust schema parity与generation升级。
4. 冻结broker核心与编码adapter边界：
   - 核心只拥有opaque profile/id、pending和connection/generation，不读取JSON业务字段；
   - `jsonrpc-2.0-text` adapter拥有request/response/error/cancel framing、string id、batch拒绝和strict
     shape validation；
   - 未来binary adapter无需复制pending owner。
5. 冻结全部生命周期和错误投影：wrong owner/generation/id、乱序、remote error、deadline、ancestor
   cancel、runtime/socket disconnect、pending/payload上限、tombstone饱和驱逐和late response。
6. 列出全部受影响production、direct fixture、README/checker，区分必须修改、负例保留和历史result不改写。
7. 给出后继实现DAG：单一shared checkpoint，随后runtime与Router互斥写集，combined probe和高风险独立
   验收面。每个节点写明首次修改、聚焦命令、遮挡关系和证据失效边界。
8. 若repo owner迫使broker核心依赖JSON、或仍有会改变公共API/wire的未决选项，返回
   `TASK_SCOPE_EXPANDED`或`TASK_NOT_EXECUTABLE`，不得在审计中实现新抽象。

## 证据与交付

使用`rg`、Cargo metadata、现有test listing与必要的只读源码追踪。最多运行确认selector非零的便宜聚焦
测试；不运行完整Rust/Router suite、live、instance或stable。

- worktree：`/Users/geek/workspace/skiff-p5-f439b-websocket-jsonrpc-audit`
- 分支：`codex/p5-f439b-websocket-jsonrpc-audit`
- result：`P5-F439B-skiff-websocket-jsonrpc-owner-audit-result.md`

新增并提交result，返回commit/tree、owner矩阵、实现DAG、验证矩阵与clean状态。完成后不得自行承接实现。

