# P5-F438A Skiff WebSocket request owner audit

状态：Ready。高风险、只读shared wire/runtime owner审计。

## 直接父节点

- `P5-F438-websocket-outbound-request-response-batch.md`

父节点已记录用户决策、权威设计commit、精确输入、当前遮挡和后续DAG。启动时只读本任务；需要依据时
沿父节点引用链向上读取。

## 输入与目的

| commit | tree |
| --- | --- |
| `64a0ab4ec85d25899dc8563ac6d647edad8ed23e` | `562adcfc8baa595969a4dd1ccd2e67c4053814b9` |

本审计必须一次列清`std.websocket.requestJsonToConnection`从source typing到Router物理socket的全部
production/test owner，避免按编译首错逐文件扩张。它不实现、不修改fixture、不运行完整gate。

## 只读范围

允许读取：

- `std/**`
- `compiler/**`中std callable、generic native、effect、suspension与required-context owner
- `runtime/**`中native dispatch、capability context、outbound pending、cancel、transport与actor executor
- `router/**`中runtime protocol/endpoint、WebSocket gateway、connection/generation索引与tests
- `test-runner/**`、`scripts/**`中直接生成或校验上述surface的fixture/checker
- 父节点及其引用链

唯一允许写入是本leaf result。禁止修改production/test/design、Internals或skiff-packages。

## 必须回答

1. 画出当前`connection.send`完整链：

   ```text
   std source/API
     -> compiler native/effect/required context
     -> eval/native dispatch
     -> host capability/control
     -> Rust runtime protocol encode
     -> Router protocol decode/runtime endpoint trust
     -> WebSocket gateway exact connection write
   ```

   对每个跳点列definition、producer、consumer、fixture、generation/schema常量和聚焦测试。
2. 找出最接近的“native发起并挂起等待外部response”现有实现，分别核对：
   - eval如何返回pending future/continuation；
   - execution deadline/cancel如何到达host operation；
   - response registry如何保证exactly-once completion与disconnect cleanup；
   - actor executor何时真实释放执行权；
   - typed request encode与response decode从哪里取得concrete generic类型。
   不得仅凭名字假定HTTP/service call owner可复用。
3. 冻结新内部wire的最小代码owner：
   - runtime-originated `connection.request`与`connection.request.cancel`；
   - Router-originated `connection.response`；
   - runtime correlation与peer-visible request id的映射边界；
   - header/payload分工、protocol strict validation和TS/Rust parity；
   - 需要原子升代的schema/generation/fixture集合。
4. 冻结Router broker owner与生命周期：
   - source runtime/service/entry/connection trust validation；
   - socket object/generation keyed pending；
   - out-of-order completion；
   - deadline/cancel/runtime disconnect/socket close；
   - bounded pending/payload/tombstone limits；
   - `1002` protocol error、`1003` unsupported peer data与late settled discard的分流。
5. 冻结std/compiler/runtime surface owner：
   - `WebSocketRequestError`与public API export；
   - generic native signature与type args/runtime type plan；
   - `std.json.DecodeError`和transport error的分界；
   - `maySuspend`、effect、target/conflict key、cancel safety与required context；
   - 普通send保持non-suspending的反向保护。
6. 列出全部会受影响的production、direct fixture、README/checker，区分必须修改、负例保留与历史
   implementation result不改写。反向搜索不能只覆盖当前首错。
7. 给出后继实现DAG：
   - 单一shared std/native+wire checkpoint的精确写集；
   - runtime与Router可并行leaf的互斥写集及依赖；
   - 每个节点首次实际修改、聚焦测试和最早风险探针；
   - combined owner与高风险独立验收面。
8. 若任一公共字段、错误语义或pending owner仍有两个会改变实现方向的选项，返回
   `TASK_NOT_EXECUTABLE`并只列最小决策问题；不要自行扩张设计。

## 证据

使用`rg`、Cargo metadata、现有test listing与必要的只读源码追踪。最多运行能确认selector非零的便宜
listing或现有聚焦测试；不运行完整Rust/Router suite、live、instance或stable。Result必须记录命令、实际
命中数、遮挡和未运行原因。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f438a-websocket-request-audit`
- 分支：`codex/p5-f438a-websocket-request-audit`

新增并提交`P5-F438A-skiff-websocket-request-owner-audit-result.md`，返回commit/tree、owner矩阵、
实现DAG、验证矩阵与clean状态。不得修改production/test、merge、rebase、push、stable/live；完成后不得
自行承接implementation。
