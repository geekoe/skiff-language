# P5-F445B Timeout expression implementation preflight result

状态：`TASK_SCOPE_EXPANDED`。

当前 reference 足以形成可执行实现 DAG，不需要补充语言设计决策；但不能把实现压成一个
“让 F444C 的三次 WebSocket 调用能 parse”的 leaf。当前 production 同时没有 source
`value` block、`concurrent` / `serial` surface、block-local execution scope、动态 host
deadline 传播和 concurrent lane scheduler。只补 lexer/parser 或只给
`requestJsonToConnection` 增加私有 timeout 参数，都会留下 reference 已承诺语义的静默错误。

因此本结论不是 `DESIGN_DECISION_REQUIRED`，也不是单 leaf 的
`PREFLIGHT_COMPLETE / TASK_EXECUTABLE`；后继应按第 5 节的七个互斥写集节点执行。

## 1. 输入和只读边界

- 任务指定的 Skiff production/reference 输入是 `c81266f3`。本 worktree HEAD 为
  `e31ad3e4b2bb0844aa701b628845d4cb80f1b3eb`；相对 `c81266f3` 只新增 F445A/F445B
  task 文档，`syntax/`、`compiler/`、`artifact-model/`、`runtime/`、`router/`、
  `std/`、当前 reference 和必要 architecture 均为零 diff。
- worktree 在预检开始时 clean，branch 为 `codex/p5-f445b-timeout-preflight`。
- F444C stash commit
  `91f3cc32e9d6ce0b14b4145d3d94815ab1a52420`、tree
  `84c76648ce7069cb44b9aa72025bb8be30827266` 仅被只读查看；没有
  apply、pop 或 drop。三项新 Host peer source 位于该 stash 的 untracked parent
  `a6f0b6d418bd4a6f74af9c6dc48a94ff951c50eb`，这也是为什么只对 stash 主 tree
  `git grep` 不会找到它们。
- 没有运行完整 gate、stable instance、live、browser 或 network 验证；本节点是 source
  audit，没有用时间敏感测试伪造 implementation receipt。

## 2. 当前 reference 已冻结的完整合同

### 2.1 Syntax、scope、type 和 effect

1. duration literal 是一个 token，由正整数和 `ms`、`s`、`m`、`h`、`d` 之一组成。
   `15s` 不能被当成 `15` 与 `s` 两个 token，也不能接受 `15 s`、`1.5s`、`0s`、
   负数或未知单位。它只允许出现在 `timeout(...)` 和平台 schema 明确开放的位置。
2. duration 常量在 compiler 中换算为 safe-integer milliseconds；单位乘法必须 checked，
   不能先经过 `f64` 再猜回整数，也不能把普通 `integer` 隐式当 `Duration`。
3. `timeout(200ms) { ... }` 是 statement，不产值；`timeout(200ms) value { ... }`
   是 value expression。普通 `value { ... tailExpr }` 必须有 tail expression，
   tail 决定表达式类型并接受外部 expected type；其 block 是词法作用域，且禁止
   `return`、`break`、`continue`。
4. canonical modifier 顺序只有：

   - `concurrent value { ... }`
   - `timeout(200ms) value { ... }`
   - `timeout(200ms) concurrent value { ... }`

   `timeout` 不能出现在 `concurrent` surface 内；第三种写法是 timeout 从外层包住整个
   concurrent value，不是 concurrent lane 内嵌 timeout。非 canonical 重排必须拒绝。
5. 捕获带 modifier 的 value expression 使用括号形态，例如
   `catch<TimeoutError>(timeout(15s) value { ... })`，不能误套用
   `catch<E> value { ... }` 简写。
6. timeout wrapper 不抹掉 body/tail 的类型、调用 effect、root provenance、
   mutation 或 `maySuspend` 分析。它不发布 service `throws` 集，也不能使本来不安全的
   concurrent external effect、外层 mutable-root 写入或 stream `emit` 合法。
7. `concurrent` 是受限直属 lane list，不是普通 block。`serial { ... }` 只能成为其直属
   lane；lane dependency、sibling-visible `const`、forward-reference、tail lane、
   effect/conflict-key、cancel-safety 和 outer-root mutation 必须在 compiler 阶段
   fail closed。

### 2.2 Deadline、timeout、cancel 和 cleanup

1. timeout 只收紧当前 block 及其中 host/remote operation 的 deadline，不能延长 caller
   request deadline，也不能在 inner timeout 被 catch 后污染或取消余下 request。
2. operation 的 effective deadline 是以下候选中最早的 monotonic absolute deadline：

   - caller request deadline；
   - enclosing timeout deadline；
   - consumer dependency timeout；
   - callee operation timeout；
   - primitive operation timeout。

3. block deadline 到达后，语义结果立即固定为可捕获 `TimeoutError`，未完成 work 收到
   structured cancellation；OS socket、数据库或 CPU 指令不要求同一机器指令内物理停止。
   不能把 ancestor cancellation 映射为 `TimeoutError`，也不存在 public `CancelError`。
4. nested timeout 取最早 effective deadline；同时到达时只有最外层 timeout 可观察。
   request deadline / outer block timeout 的确定事件优先于 concurrent lane error；
   primitive/API timeout 和用户手工 `throw TimeoutError` 只是普通 lane error。
5. 纯 Skiff CPU 必须有界响应。最低 checkpoint 包括 function entry、loop condition 前、
   loop backedge、每个 lane 开始前/结束后、`concurrent value` tail lane 前和长生成片段。
6. winner 固定后，未启动 lane 不再启动，运行中 lane 被 cancel；late value、late error
   和 cancel 后的 Skiff-visible write 不得重新决定结果。已提交外部副作用不回滚。
   不支持底层 cancel 的 operation 进入有 grace/platform limit 的后台 cleanup，外层不等待。
7. stream 的 `break`、`return`、outer timeout 和 ancestor cancel 都必须向 source 传播
   cancel；operation metadata 的 commit point、cancel-safety、idempotency、cleanup action
   和 lower-cancel 能力仍然生效。

### 2.3 WebSocket 只是合同的一个 consumer

`std.websocket.requestJsonToConnection<TRequest,TResponse>` 继续保持三个参数：
`connectionId`、`method`、`value`。它继承当前 execution deadline/cancel；deadline 是
`TimeoutError`，ancestor cancel 不可捕获。二者都必须先原子删除 pending，再
best-effort 发 `$/cancelRequest`；late response 不能恢复调用。

这不允许新增 WebSocket-only 15 秒参数，也不允许用 F444C deployment 的 120 秒 request
timeout 代替 block-local 15 秒 deadline。

## 3. Production gap 与可复用机制

| 层 | 当前可复用事实 | 当前缺口 / production owner |
| --- | --- | --- |
| lexer/parser/AST | parser 已按 statement/expression slot 分流，keyword 多以 `Ident` spelling 判断 | `syntax/src/lexer.rs` 只有 `Number(f64)`/`Ident`，`15s` 被拆开；`Stmt`/`Expr` 没有 value、timeout、concurrent、serial；parser 和 `ast_utils` visitor 都没有路径 |
| source semantics | 现有 expected-type、flow、effect、root provenance、mutation、suspend analysis 可扩展 | `compiler/source/**` 的 exhaustive walkers/type/effect rules 没有这些节点，也没有 lane DAG、sibling const、modifier 和 concurrent-surface rejection |
| value block | artifact `ExprIr::ValueBlock`、linked mirror 和 eval sequential path 已存在 | 它只由 DB lowering 内部生成；source AST/parser/type/flow 不支持用户 `value {}`，因此不是现成的语言 feature |
| compiled IR | `InstructionSourceSite`、block/expression tables、canonical File IR identity 已存在 | `StmtIr`/`ExprIr` 没有 timeout/concurrent plan；lowering、external/publication ref walkers、emission、linker 都缺 exhaustive case |
| artifact | File IR 是 tagged、`deny_unknown_fields`、canonical-hashed artifact | 新 executable kind 会改变 File IR wire schema和 file/package build identity；不能塞进 metadata 或丢掉 source site |
| request deadline | `runtime/request::ExecutionBudget` 已用 monotonic `Instant` 合并 request `timeoutMs`/`expiresAt`；instruction counter、poll interval和 first-failure stats 已存在 | deadline 是 request 构造时不可变；没有可派生、可 catch 后退出而不污染 parent telemetry/cancel state 的 local scope，也没有 deadline source/depth tie-break |
| cancellation | notify-backed `CancellationSource/Token` 与多 signal wait (`CancellationSignals`) 可复用 | `ExecutionControl` 只持一个 token和一个 request budget；没有 parent/local 分源的 child scope，不能区分 ancestor cancel 与 local timeout cancel |
| eval/checkpoint | statement/expression/多处 loop 已调用 instruction/poll；ordinary catch 已把 non-cancel budget failure投影为 builtin `TimeoutError` | 没有 timeout frame、scope rebinding、lane scheduler、deterministic winner、tail lane和完整 checkpoint audit |
| request heap | `RequestHeap: Clone`，并已有 `deep_clone_runtime_value_carrier_between_heaps` | 当前 evaluator 独占 `&mut RequestHeap`/`Env`，不能直接真实重叠 sibling future；concurrent 必须使用 lane-local heap/env并只在 normal join 时导入可见 const/tail |
| service outbound | dependency/operation timeout min、request lease、deadline wait和 cancel frame已有 | `OutboundCallerDeadline` 从 request `extra` 一次性快照，local timeout不可见 |
| HTTP/host/time/file | HTTP transport已有 primitive timeout与 frame deadline min；time/file已有 ExecutionControl-aware部分 | HTTP effect context在 request adapter构造时快照 deadline；operation invocation缺 current scoped control，stream/cleanup也未统一重绑 |
| outbound WebSocket | `ConnectionRequestRegistry` 已实现 pending atomic settle、timer/lease accounting、cancel notification和 late response discard；native dispatch已区分 deadline/cancel | host adapter用 request初始 token/deadline构造 WebSocket context；local timeout无法传入 registry |
| `runtime_websocket_jsonrpc` | whole inbound JSON-RPC request已有 `tokio::select!` terminal race、cancel-before-deadline优先和 late handler result不写 response的测试 | 它是 peer-initiated inbound gateway request supervisor，不是 source timeout或 outbound request owner；只能复用 race/terminal test pattern，不能直接调用来实现 block-local timeout |

现有 `ExecutionControlError::BudgetExceeded` 到 `TimeoutError` 的 catch projection和
`RuntimeError::Cancelled` 的不可捕获分离应保留。local timeout 不能调用 shared request
budget 的 `record_deadline_exceeded()`，也不能 cancel shared request token；前者会把
block-local winner错误记成 request-wide first-failure telemetry，后者会使 outer catch
之后的 parent execution变成不可捕获 cancellation。

## 4. 为什么不能是一个 bounded leaf

即使只看 F444C spelling，parse 成功也至少跨 syntax、source type/effect、lowering、File IR、
linked IR、eval和 host adapter。完整 reference 又明确要求
`timeout(...) concurrent value`、lane deadline precedence和 concurrent-surface rejection，
而当前 production 没有任何 source/runtime concurrent 实现。

把所有工作放进一个 leaf 会同时拥有：

- language grammar与 diagnostics；
- static lane/effect/mutation analysis；
- persisted executable schema与 identity generation；
- request/cancellation primitive；
- evaluator heap/frame/scheduler；
- HTTP/service/WebSocket host propagation；
- hermetic cross-layer receipts。

这些不是一个可独立审阅、可回滚的写集。正确停止点是 scope expansion，而不是提交只覆盖
Agine happy path 的半语义实现。

## 5. 精确实现 DAG

```text
I1 syntax
 ├──> I2 source/type/effect/concurrent DAG ──> I3 artifact/lowering/linking ──┐
 └──> I4 scoped execution control ────────────────────────────────────────────┤
                                                                              v
                                                                  I5 eval/lane runtime
                                                                              |
                                                                              v
                                                                  I6 host propagation
                                                                              |
                         I1 + I2 + I3 + I4 + I5 + I6 ──────────────────────────┘
                                                                              v
                                                                  I7 closure/receipt
```

I2/I3 与 I4 可以在 I1 的 duration representation冻结后并行；I5 必须等待 I3 和 I4；
I6 必须等待 I5 的 current-scope invocation contract；I7 等待全部 production 节点。

| 节点 | owner | 唯一写集 | 交付与退出条件 |
| --- | --- | --- | --- |
| `P5-F445B-I1` | Syntax owner | `syntax/**` | 新 `DurationLiteral` token保留原始 digits/unit/span并checked换算；新增 source AST value/timeout/concurrent/serial节点、canonical modifier parser、visitor；parser正反例 GREEN |
| `P5-F445B-I2` | Compiler source-semantics owner | `compiler/source/**`、`compiler/driver/pipeline/mod.rs` | statement/value typing、expected type、flow禁止项、lexical scope、body effect/maySuspend透传；完整 concurrent surface、sibling const、lane dependency、mutation/effect/cancel-safety检查；所有未知路径 fail closed |
| `P5-F445B-I3` | Artifact/lowering/link owner | `artifact-model/**`、`compiler/core/src/spawn_targets.rs`、`compiler/compiled/**`、`compiler/lowering/**`、`compiler/emission/**`、`compiler/projection/**`、`runtime/linked-program/**`、`runtime/linker/**` | 显式 `Timeout` stmt/expr wrapper（checked ms + source site）、可复用 user `ValueBlock`、compiled `ConcurrentPlanIr`（lane source order/kind/dependencies/tail）；所有 IR walker/link validation完整；File IR version/identity/golden原子更新 |
| `P5-F445B-I4` | Request/cancellation owner | `runtime/request/**`、`runtime/capability-context/**` | `EffectiveDeadline { at, source, nesting }` 和 derived execution scope；request stats/instruction counter共享，local deadline terminal独立；parent/local cancel分源；earliest/tie、ancestor不可捕获、catch后parent继续、timer/lease零泄漏测试 GREEN |
| `P5-F445B-I5` | Eval/concurrency owner | `runtime/eval/**`、`runtime/model/**` | sequential statement/value timeout、current scope安装/恢复、CPU checkpoint；lane-local heap/env + normal join导入、真实 async overlap、DAG调度、deterministic winner、tail lane、late result/write discard、stream cancel；定义供 host adapter使用的 invocation-time scope contract |
| `P5-F445B-I6` | Host/native owner | `runtime/host/src/**`、`runtime/native/src/**` | HTTP、service outbound、WebSocket、time/file/stream operation在调用开始时读取 I5 current scope，不再只读 request-start snapshot；effective min、wire deadline、lower cancel、cleanup/late response receipts GREEN；std WebSocket签名仍为三参数 |
| `P5-F445B-I7` | Cross-layer conformance owner | `compiler/tests/**`、`runtime/eval/tests/**`、`runtime/host/tests/**`、`runtime/transport/tests/**`、`runtime/driver/eval/tests/**`、`runtime/package-test/tests/**`、本 phase 对应 implementation result | 第 6 节全部 hermetic RED→GREEN；artifact version/identity receipt、无 legacy spelling reverse-search和 F444C source compile probe通过；不以真实网络或 stable instance代替 fixture |

I3 的推荐 executable composition 是：

- statement form：`StmtIr::Timeout { duration_ms, block, site }`；
- value form：`ExprIr::Timeout { duration_ms, value, site }`，其中 `value` 可以是现有
  `ValueBlock` 或新的 `ConcurrentValue`；
- concurrent form：compiler产出已验证的 lane DAG，runtime不重新猜 source dependency；
- `serial` 作为单个 lane的有序 body进入 plan，不需要把非法 source surface带到 runtime。

这让 `timeout(...) concurrent value` 表示外层 timeout wrapper，而不会创造“concurrent
surface 内允许 timeout”的第二套语义。

I4/I5 的最低实现不应复制一个新的完整 request budget。应共享 request instruction
statistics/limit，叠加独立 local deadline和local cancel source；deadline winner携带 source
site/nesting，等时选择 outer source。lane evaluator使用已有 cross-heap clone/import能力，
只把 normal-completed sibling-visible const和最终 tail导回 coordinator heap；取消后的
lane-local mutation天然不会泄漏。

## 6. 独立于 Agine 的最小 RED/GREEN matrix

所有异步用例使用 paused/fake monotonic clock、barrier和 fake capability，不使用 wall-clock
长 sleep 或真实网络。

| ID | 建议 fixture / 层 | 当前 RED | 必须达到的 GREEN |
| --- | --- | --- | --- |
| `T01` | `syntax` duration lexer/parser | `15s` 为 Number + Ident，timeout无法 parse | `1ms/1s/1m/1h/1d` 单 token；`0s`、`-1s`、`1.5s`、`15 s`、`1x`、safe-ms overflow和 timeout外使用均给稳定 diagnostic |
| `T02` | `compiler/tests/timeout_source.rs` statement | AST无节点 | `timeout(20ms) { const x = 1 }` normal completion且不可赋值 |
| `T03` | 同上 value typing | source `value` 不存在 | `const x: string = timeout(20ms) value { const y = "x"; y }` 类型为 string；缺 tail、tail mismatch、return/break/continue均拒绝 |
| `T04` | 同上 modifier grammar | concurrent/value均不存在 | 三种 canonical form接受；重排拒绝；concurrent surface内 timeout、普通 value、control/catch/emit/spawn等全部拒绝 |
| `T05` | `runtime/eval/tests/timeout_execution.rs` normal/value | 无 linked timeout | fake clock deadline前返回 statement normal/value tail，scope退出后parent deadline恢复 |
| `T06` | 同上 throw/catch | 只有 request budget catch projection | `catch<TimeoutError>(timeout(15ms) value { ... })` 得到 err；source site/budget reason稳定，catch后后续 parent code正常运行 |
| `T07` | 同上 nested earliest | 无 nested scope | inner早则inner observable；outer早则outer observable；同一 absolute deadline只 outer observable |
| `T08` | 同上 request earlier | local scope不可表达 | request deadline早于 local时取 request deadline；local绝不延长 request |
| `T09` | 同上 ancestor cancel | 单 token | ancestor cancel在 timeout/catch内外都保持 internal terminal，不能成为 `CatchResult<_,TimeoutError>`；cancel/deadline同 ready时保留现有 cancel-first terminal invariant |
| `T10` | 同上 pure CPU | 只有零散 request poll | timeout包住无 host await的长 loop，在有界 checkpoint数内结束；function entry、condition、backedge和generated chunk计数均被断言 |
| `T11` | 同上 concurrent lanes | 无 scheduler | 两个 barrier-controlled async read真实重叠；DAG dependency按序；tail等待所有前序 normal exit；source-order lane error与outer timeout优先级确定 |
| `T12` | 同上 cleanup/late | 无 block owner | winner后未启动lane为零启动，运行lane收到cancel；late value/error/heap mutation不导入；cleanup lease/timer/pending最终为零 |
| `T13` | `runtime/host/tests/timeout_deadline_propagation.rs` HTTP | request-start snapshot | fake HTTP看到 `min(request, local, primitive)` absolute deadline；deadline后response被丢弃并触发声明的 cleanup |
| `T14` | 同上 service outbound | caller deadline从 request extra快照 | child service frame携带 tighter local deadline；dependency/callee operation更早时仍获胜；cancel frame只发一次 |
| `T15` | 同上 outbound WebSocket | pending registry可用但收不到local scope | 三参数 request安装 tighter deadline；timeout/cancel先原子移除pending，再发一次 `$/cancelRequest`；late response返回 false且不能恢复别的调用 |
| `T16` | stream/time/file fake capability | 部分使用 request ExecutionControl | sleep、file source和stream consumer继承local scope；break/return/timeout/cancel传播到source，不支持lower cancel时进入bounded cleanup |
| `T17` | artifact/link/identity golden | IR无新 kind | new kinds strict round-trip；unknown/legacy schema fail closed；compiler/linker exhaustive；source相同输出canonical稳定 identity |

每个 production 节点先提交其本层 RED，再实现 GREEN。I7 不能用只编译 F444C 三段 source
替代 `T01`–`T17`。

## 7. Artifact、schema、identity、fixture 和 receipt 影响

### 必须变化

- `FileIrUnit` executable wire schema新增 tagged kind。当前
  `FILE_IR_SCHEMA_VERSION=skiff-file-ir-v8`、`FILE_IR_FORMAT_VERSION=skiff-file-ir-format-v6`；
  实现节点应原子提升为 v9/v7。新增 executable opcode kind时
  `FILE_IR_OPCODE_TABLE_VERSION` 也由 v1 提升为 v2，不能让新旧 runtime误读。
- File IR canonical payload和 `file_ir_identity` prefix/hash随之改变；包含 timeout/value/
  concurrent代码的 package/service build identity会改变。artifact-model、lowering identity、
  compiler emission/projection、linked-program/linker、runtime admission和 package-test fixture
  中硬编码 version/hash都要更新。
- compiler source fixture、IR JSON round-trip、link validation、runtime eval fake-clock、
  host propagation、pending/cleanup accounting和 implementation result receipt需要新增。
- F444C 恢复后的 compiled service artifact、build identity与相应 receipt必须重算，不能沿用
  stash草稿中的旧 build结果。

### 不应变化

- `std.websocket.requestJsonToConnection` public/native签名不变，仍为三参数。
- timeout不新增 service API字段、PackageSchema type、ServiceContract、
  service protocol identity、deployment schema或业务 correlation identity。
- 既有 builtin `TimeoutError` identity和ordinary catch envelope可复用；不新增
  `CancelError`或 WebSocket-private timeout error。
- Package Local ABI不因“存在 timeout wrapper”自动变化；`maySuspend`仍按 body/call graph
  推导。具体 public callable的 body改动会改变 package build identity，只有其推导出的
  public `maySuspend` summary实际变化时才改变 Local ABI。
- `RUNTIME_ASSEMBLY_SCHEMA_VERSION` 和 package artifact顶层 shape没有因为该语义自动变化；
  如果实现选择把新字段直接加入这些持久 DTO，必须在 I3 中另行原子 version，而不能静默写入。

## 8. F444C 恢复时的精确 source spelling

stash 中三个调用位于
`agine/service/internal/host_peer_rpc.skiff`，方法分别是
`host.files.list`、`host.files.search`、`host.current-directory`。保留现有 decode /
WebSocket / timeout 三层错误分类时，timeout必须包住 WebSocket catch expression，外层
`catch<TimeoutError>`使用括号形态。

list：

```skiff
const decoded = catch<std.json.DecodeError>(
  catch<TimeoutError>(
    timeout(15s) value {
      catch<std.websocket.WebSocketRequestError>(
        std.websocket.requestJsonToConnection<
          root.internal.host_peer_protocol.HostFilesListParams,
          root.internal.host_peer_protocol.HostFilesListResult
        >(
          connectionId,
          "host.files.list",
          root.internal.host_peer_protocol.HostFilesListParams {
            path: requestPath,
          }
        )
      )
    }
  )
)
```

search：

```skiff
const decoded = catch<std.json.DecodeError>(
  catch<TimeoutError>(
    timeout(15s) value {
      catch<std.websocket.WebSocketRequestError>(
        std.websocket.requestJsonToConnection<
          root.internal.host_peer_protocol.HostFilesSearchParams,
          root.internal.host_peer_protocol.HostFilesSearchResult
        >(
          connectionId,
          "host.files.search",
          root.internal.host_peer_protocol.HostFilesSearchParams {
            path: requestPath,
            query: query,
          }
        )
      )
    }
  )
)
```

current-directory：

```skiff
const decoded = catch<std.json.DecodeError>(
  catch<TimeoutError>(
    timeout(15s) value {
      catch<std.websocket.WebSocketRequestError>(
        std.websocket.requestJsonToConnection<
          root.internal.host_peer_protocol.HostCurrentDirectoryParams,
          root.internal.host_peer_protocol.HostCurrentDirectoryResult
        >(
          connectionId,
          "host.current-directory",
          root.internal.host_peer_protocol.HostCurrentDirectoryParams {}
        )
      )
    }
  )
)
```

不能写成第四个 WebSocket参数、`timeoutMs: 15000`业务字段、service轮询、
`Duration.seconds(15)`替代冻结 literal，或依赖 `config.dev.yml` 的120秒 request timeout。

## 9. F444C 解除条件

F444C stash只有在以下条件同时满足后才能恢复：

1. I1–I7 已按依赖进入 F444C 使用的 Skiff integration，`T01`–`T17` 的 hermetic
   receipts全绿，且实际 compiler/runtime来自同一新 File IR schema generation。
2. F444C 的另一个独立 blocker——package callable dependency-local 与 package-global
   canonical interface identity——也已进入对应 Skiff / packages / Internals integration；
   timeout落地不解除该 identity blocker。
3. stash commit及其 untracked parent仍可解析，Internals恢复 worktree clean；再由 F444C
   owner恢复 stash，不在本预检节点 apply/pop/drop。
4. 三项调用改为第 8 节精确 spelling，且 reverse-search确认没有 WebSocket私有 timeout参数、
   service polling或120秒deployment timeout替代物。
5. 重新运行 F444C 规定的 canonical `npm run type-check`、service `.test.skiff`
   success/auth/error/timeout/cancel matrix、三个 Node receipt/architecture entrypoint、
   F444A §6 reverse-search closure；若最终改动触及 Agine chat链路，再按 workspace合同运行
   chat smoke。

满足这些条件后，F444C 才能从 `TASK_SCOPE_EXPANDED` 重新建立 terminal candidate；本预检
没有宣称它已解除阻塞。
