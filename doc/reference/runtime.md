# Skiff Runtime Reference

本文负责：

- 定义 Skiff runtime 的稳定执行边界：gateway / router / service runtime 各自承担什么。
- 定义 request frame、heap value、mutable root、concurrent lane、join、timeout、cancel、stream 和错误选择的运行时语义。
- 定义 HTTP / WebSocket entry 与 service-to-service call 如何进入同一类 typed dispatch。
- 说明 effect metadata 在 runtime 中如何参与并发、取消、timeout、观测和测试替身。
- 列出当前明确不支持的 runtime 能力。

本文不负责：

- 不定义语法、类型推导、完整 schema closure、std / prelude API signature。
- 不复制 manifest、service config、transport envelope 或 std API 字段表。
- 不写实现迁移计划、artifact 文件格式、部署拓扑或数据库 adapter 细节。
- 不保留旧实现兼容语义；Skiff 尚未发布，本文按目标语义收敛。

## 1. Runtime boundary

Skiff runtime 是一组边界职责，不是用户源码可访问的全局对象。

Gateway adapter 负责外部协议适配：接收 HTTP、HTTP stream / SSE、WebSocket 等入口，维护外部连接，执行协议层 decode / encode，把外部入口转换成 router 可路由的 typed dispatch，并把 unary response、stream chunk、stream end 或 error 编码回外部协议。Gateway 不执行用户 Skiff 代码，不拥有 Skiff call stack 或 request heap。

Hub / router 是独立于用户 service runtime 的平台基础设施。它负责 service runtime 注册、service revision 注册、可用实例选择、service protocol identity / ingress entry identity 匹配、client session / actor binding / WebSocket Connection 索引、in-flight request / stream 配对、cancel / drain 路由、负载和流量切换。Router core 不解释业务 host、path、cookie、session、应用 WebSocket eventName 或业务 requestId。

Service runtime 是执行用户 Skiff 代码的边界。它加载已发布 artifact，构造 service revision singleton，为每次 dispatch 创建 request frame，解码 payload，调用 implementation method 或 entry handler，执行表达式、函数、collection mutation、`concurrent`、`timeout(...)`、`emit` 和 cleanup，并编码 response / chunk / error。

Service runtime 不维护外部 WebSocket 物理 socket 生命周期。WebSocket Connection 属于 gateway / hub；connect、receive 或 route dispatch 只是进入 service runtime 的一次 request。

生产 runtime 的 artifact 信任边界是平台 build service。Runtime 不把开发者本地编译产物视为线上发布权威；线上 artifact 必须由平台 build service 产生、签名并记录 provenance。

## 2. Dispatch and request frame

以下事件创建 request frame：unary service API call、server-stream service API call、raw HTTP entry dispatch、WebSocket connect operation、WebSocket receive / route dispatch，以及测试 runner 构造的等价 dispatch。

Gateway / router 可以维护 entry envelope、routing context、transport state、Connection 和 stream pairing state，但这些不是 Skiff request frame。

Request frame 包含 request 参数、request context、deadline、trace、cancel state、Skiff call frames、slot values、request-local heap、exception envelope、`concurrent` lane state、join state、server-stream sink 或 external stream source handle，以及 request 内创建的 mutable root / resource handle。

Request frame 不包含外部 WebSocket 连接生命周期、跨 request 业务状态、持久化数据库状态或 service revision singleton。

Unary request 在 response end、response error、timeout 或 cancel 后结束。Server-stream request 在 stream end、stream error、timeout 或 cancel 后结束。Server-stream request 仍是一段有限 request 生命周期，不是 WebSocket connection，也不是后台任务。

Request 结束后，request heap、call frames、slot values、lane state、exception envelope 和 request-local stream / resource handle 全部清理。Heap handle、`Exception<E>`、`CatchResult<T, E>` 和 request-local stream 不能逃逸到 request 结束之后。

## 3. Runtime transport model

Runtime 内部 transport 只表达 typed operation dispatch，不把 raw socket、raw WebSocket frame、raw SSE event 或宿主语言 HTTP object 暴露给 service runtime。

Unary dispatch 最多产生一个 final response：成功时 response end 携带 payload，失败时 response error 携带 runtime error envelope；unary dispatch 不能产生 stream chunk。

Server-stream dispatch 产生有序 chunk 序列，最后产生一个 end 或 error。普通 service stream 中，每个 `emit` 对应一个 response chunk；normal exit 对应无 payload 的 stream end；throw、remote error、decode error、timeout 或 cancel 对应 stream error 或 cancel 传播。HTTP response stream 额外先发送 `response.start`，再发送 `response.chunk`，最后发送空 `response.end`。Stream end 后没有额外 final aggregate response。

Payload 在进入 service runtime 前按 expected schema decode，离开 service runtime 时按 response schema encode。JSON 只是显式 codec 的一种，runtime boundary 不把裸 JSON DOM 当作默认 payload 语义。

Cancel 是明确 runtime 事件，不是让 in-flight request 静默消失。断线、caller cancel、timeout、consumer 提前停止 stream 迭代和 drain 都必须收敛为 cancel 或 error 语义。

默认不自动 retry。只有 effect metadata 和 operation schema 明确声明幂等、可比较 target / conflict-key，并由平台策略允许时，router 才能重试。

## 4. Runtime identities

Runtime 依赖稳定 identity 做路由、drain、观测和测试替身匹配。

Service protocol identity 描述 service-to-service API 的公开协议。API 参数 / 返回类型、operation 集合和 public schema closure 的规范化 schema 变化会改变它。跨 service call 的寻址坐标是 service id + version：caller 在依赖约束里声明被调 service 的 id 和 version，router 在请求时把 (service id, version) 解析为该 version 当前的 build 并路由到对应实例。发布时冻结的 build id 与 protocol identity 不是 release selector，而是边界兼容性 witness——dispatch 时 router 校验所解析当前 build 的 protocol identity 是否满足 caller 冻结的期望，不满足则以明确错误失败，绝不静默路由到不兼容的 build。

Ingress entry identity 描述 schemaful ingress entry。WebSocket connect request schema、message event schema、Connection context schema 和系统接口绑定变化会改变它。更改 ingress schema 不改变 service protocol identity。

Stable target id 描述 operation 类别，而不是资源实例。Service operation、HTTP entry、WebSocket connect / receive / close、std host operation 和 package wrapper 都必须能映射到稳定 target。Target 用于 effect metadata、timeout source、trace、日志、指标、测试替身和错误聚合。

HTTP raw dispatch 没有 per-route entry identity；它使用 HTTP entry target 进行观测。WebSocket Connection 绑定 entry identity 和 service protocol identity；schema-changing 发布后，旧 Connection 继续使用旧 entry identity，直到 drain 或断开。

## 5. Request heap and values

`null`、`bool`、`number` / `integer` 表现为值语义。`string` 和 `bytes` 对用户表现为不可变值；实现可以选择 inline、shared buffer 或 heap 优化，但不得暴露可变 alias。

`Array<T>`、`Map<K, V>`、`JsonObject`、record / object 和 opaque resource handle 是 heap value。赋值和传参复制 handle，不 deep clone。需要独立副本时必须显式 clone。

`const` 只限制当前 binding 不能重新绑定，不表示指向的 heap value 深度不可变。`const` 指向的 collection、record / object 或 mutable handle 仍可通过 mutating API 修改。

每个 request 创建 request-local heap。Request 参数、DB / HTTP / service call / external source 返回值、array / map / object / record literal、clone 结果、需要 heap 表达的 nominal / representation wrapper、request-local resource handle 和 external stream source handle 都分配在该 heap 中。

Request 结束时 heap 整体释放。Heap handle 是 request-local id，不是跨 request、跨 artifact 或跨 service 的 ABI。

Mutable root 是一次可变状态的语言层身份。`let` binding、record / object、`Array<T>`、`Map<K, V>`、`JsonObject`、runtime / package API 标记为 mutable handle 的 opaque resource，以及未来声明为 mutable handle 的 cursor / transaction / stream 都携带 mutable root。普通 scalar、immutable descriptor 和不含 mutable root 的临时表达式不携带 mutable root。

Root identity 不等于词法名。`const b = a` 复制引用并继承 `a` 的 root provenance；字段访问继承对应子路径 provenance；clone API 返回 fresh root；无法证明 provenance 的 mutable return 按 opaque root 处理。

Collection 和 object mutation 是原地 mutation。Runtime 沿编译后的 mutable access path 定位 heap node，检查目标类型，再执行短同步写入。第一版 request heap 不支持 cycle；会形成 cycle 的 mutation、materialize、wire payload 或 clone 必须失败。

`Map<K,V>.keys()` 分配一个 fresh `Array<K>`，内容是调用时 map key 的快照。修改返回数组不影响原 map；调用后修改原 map 也不改变该数组的元素集合。

map `for` 循环在循环开始时读取快照。`for key in map` 读取 key 快照；循环期间对 map 执行 `set` / `delete` 不改变本轮将访问的 key 集合。`for key, value in map` 读取 entry 快照；循环期间对 map 执行 `set` / `delete` 不改变本轮 key/value 对，若某个尚未访问的 key 被重新 `set`，后续迭代的 `value` 仍是循环开始时的 value。

map key 快照顺序是 canonical map key order，不是插入顺序。当前合法 map key 是 `string` 或 string representation，排序按 canonical string payload 的 UTF-8 字节序升序；未来如果扩展非 string key，runtime 必须先为该 key 类型定义 canonical map ordering。

string representation map key 在 request heap 中按 erased string payload 保存。`Map<UserId,V>.keys()` 的运行时数组元素仍是 string payload；静态类型和 boundary 编码通过 expected `Array<UserId>` schema 保留 `UserId` 身份。运行时不得把该结果重新推断成 untyped `Array<string>`。

## 6. Concurrent lane model

`concurrent` 是结构化并发语义。无依赖且通过 effect / mutation 检查的 sibling lane 必须能在 async host / service await 边界真实重叠执行；实现可以重排无依赖 lane，但不能把 `concurrent` 降级为纯串行执行。用户可见 join、错误选择和 mutation 规则必须确定。

`concurrent { ... }` 只把被修饰 block 的第一层直属项划分为 lane：直属 statement 是一个 lane，直属 `serial { ... }` 整体是一个 lane，`concurrent value { ... }` 的 tail expression 是保留 `tail` kind 的普通 synthetic lane。当前 `concurrent` surface 是受限 lane list，不是普通 block；`if`、`match`、loop、`with`、`timeout`、普通 `value` block、`return`、`break`、`continue`、直接 `throw` / `rethrow`、`catch`、`emit`、`spawn`、嵌套 `serial`、嵌套 `concurrent` 和 callback / anonymous function body 在该 surface 内非法，包括在直属 `serial { ... }` 内非法。被调用函数内部仍可包含普通控制流；lane 只观察其 normal return、throw、timeout 或 cancel 结果。

`concurrent` block 自身是词法作用域，但只有直属 const declaration lane 声明的 `const` 能被后续 sibling lane 读取。后续 lane 只能读取源码位置严格在前的 sibling-visible `const`。读取后方声明是 forward reference。当前 `let` 在 `concurrent` surface 内非法；嵌套 block 内声明和 `serial` 内声明都不跨 sibling 可见。

Compiler 为每个 `concurrent` block 建立 lane DAG。Lane B 读取 lane A 的 sibling-visible `const`，则 A 必须先于 B 完成。Tail lane 依赖它读取的前序 const，也隐式依赖源码位置在它之前的所有 lane normal exit。外层 mutable root 写入不是依赖边；当前直接禁止。

当前，`concurrent` sibling lane 禁止对外层 mutable root 产生 Skiff 可见写入。该限制按 root provenance 判定，而不是按词法名判定。即使两个 sibling lane 写不同字段，只要 root 来自 `concurrent` 外层，也不允许。Lane-local fresh root 可以在该 lane 内 mutation。`serial { ... }` 只收束顺序逻辑，不绕过外层 mutable root 写入限制。

`concurrent` block normal completion 前，所有已启动 lane 都必须 normal exit。某个 lane error、外层有效 timeout 或 ancestor cancel 确定获胜事件后，尚未启动的 lane 不再启动，正在运行的 lane 收到结构化取消信号，被取消 lane 后续产生的值、错误和 Skiff 可见写入被丢弃。已提交外部副作用不回滚，只能依赖 effect metadata、幂等、日志和补偿策略治理。

当前不定义 detach lane 或未归属后台 lane。

## 7. Error selection

Block 的退出结果属于普通完成、正常控制退出、错误退出、timeout 退出或结构化取消。`return` / `break` / `continue` 是正常控制退出；`throw` / `rethrow` 是错误退出；block-level `timeout(...)` 产生 timeout 退出。结构化取消不是用户可捕获的普通异常。

同一个 `concurrent` block 中，用户可见错误选择必须确定：外层 `timeout(...)` 或 request deadline 形成的有效 timeout 优先于 lane error；多个 sibling lane error 同时成为候选时，源码位置靠前的直属 lane 获胜；嵌套 timeout 同时到达时，只最外层 timeout 产生可观察事件。当前 `timeout(...)` 不能出现在 `concurrent` surface 内。

外部 API operation timeout 是该 API 所在 lane 的 lane error，不享受 block-level timeout 的最高优先级。用户手工抛出的 `TimeoutError` 也是普通 lane error。

获胜事件确定后，外层 `catch` 只能捕获该事件对应的 exception envelope。其他 lane 后续错误只能进入平台日志 / trace。

服务 API 不在函数签名上声明业务 `throws`。预期内业务失败应收敛为返回类型，例如 named union 或 discriminator record union。未捕获并越过服务边界的业务 error leaf 会被 runtime 转换为平台级未处理服务错误。

Service-to-service 和 gateway-to-runtime 的 response error 在 caller 侧恢复为普通 throw envelope；`catch<E>` 不区分本地 throw 与远程 platform error throw。Ingress decode 在进入 service operation 前失败时，业务代码尚未运行，不能被业务 `catch` 捕获。

## 8. Timeout and cancel

每次 request 在一个有效 deadline 下执行。`timeout(...)` block 只能收紧当前代码块和其中远程 / host 调用的有效 deadline。

一次远程调用或 host operation 的可见 deadline 是 caller request deadline、外层 `timeout(...)` deadline、consumer dependency timeout、callee operation timeout 和 primitive operation timeout 中最先到达者。

Runtime 使用单调时钟计算内部 absolute deadline。该 absolute deadline 不暴露给用户代码；用户可见的是 `TimeoutError` 的 budget / source 语义。

Deadline 到达且 block / request 尚未结束时，对应语义结果立即固定为 `TimeoutError` 或平台 timeout error，未完成 work item 收到结构化取消信号，外层代码不等待后台清理完成，后台清理产生的值、错误和 Skiff 可见写入被丢弃。“立即”表示语义结果立即确定，不表示 OS socket、数据库请求或纯 CPU 指令在同一个机器指令内物理停止。

Runtime / compiler 必须让纯 Skiff CPU 代码可被有界取消。取消检查至少出现在函数入口、loop 条件求值前、loop backedge、每个 lane 开始前和完成后、`concurrent value` tail lane 开始前，以及可能长时间运行的生成代码片段中。

结构化取消会把当前 request/lane 固定为 `CancelError`，可被显式 `catch<CancelError>` 捕获；当前没有用户可调用的 runtime cancel inspection API。

Host operation 必须通过 metadata 声明 commit point、cancel-safety、idempotency、cleanup action 和是否支持底层取消。Commit point 之前取消且 API 声明 cancel-safe 时，runtime 可以保证无外部副作用。Commit point 之后取消时，外部副作用可能已经发生；语言层只丢弃返回值或错误。不支持底层取消的 host operation 必须由 runtime 托管到后台清理路径，并受 grace period / platform limit 约束。

## 9. Stream semantics

Skiff 当前只支持 server / source stream。Stream 是一次性值，不是持久化数据结构。

`Stream<T>` 可以出现在 service operation 或 ingress entry 绑定 operation 的最外层返回类型，表示 server stream；也可以作为显式 stream-producing native std / package API 的返回类型，表示 request-local external source handle。平台 std 可以返回包含 stream 字段的 runtime-owned handle record，例如 `std.http.HttpClientStreamHandle.body`；这个 record 只能在当前 request 中使用，不能持久化或作为业务协议 schema。

`Stream<T>` 当前不能作为用户 operation 参数、用户 record 字段、持久化字段、collection 元素或普通 public API type 字段。平台 std 的 runtime-owned handle 字段和 native host operation 参数是特权例外；例如 `std.file.createFromStream(source: Stream<bytes>, ...)` 在同一 request 内消费 source，不把 stream 传出为远程 API 或 durable value。普通 Skiff package / local function 不能通过源码 body 创建独立、可逃逸的 stream source。

返回 `Stream<T>` 的 service / ingress entry operation 是 server-stream producer。Producer 共享当前 request frame、deadline、trace、call stack 和 request heap。函数体内 `emit expr` 要求 `expr` 可赋给 `T`，并向当前 stream sink 写一个 ordered chunk。函数体自然结束或裸 `return` 表示 stream normal end；`return expr` 在 server-stream operation 中是编译错误。当前不提供 `Stream<T, R>` 或 stream 完成后的独立 response 值。

`emit` 是 backpressure point。Consumer 不读取、gateway / client 断开或 buffer 达到平台上限时，当前 request 必须暂停、取消或按平台错误结束，不能无限积压。`emit` 不允许出现在 `concurrent` surface 内；`concurrent value` 的 tail lane 也属于该 surface。需要并发计算后输出时，先在 lane 中计算值，等 `concurrent` block 结束后，在后续顺序代码中按确定顺序 emit。

调用方把 `Stream<T>` 当作只能顺序消费的一次性值。每次迭代读取下一个 item；chunk 产生一次 loop body 执行；end 使 loop normal exit；error 映射为当前 lane 中的 ordinary throw，可被外层 `catch<E>` 捕获。已经处理过的 chunk 不回滚。

`break`、`return`、timeout 或 ancestor cancel 必须向 stream source 传播 cancel。Stream 被消费、结束或取消后不能再次迭代，也不能复制到多个 lane 同时消费。

跨 service / gateway stream 使用 runtime transport 的 stream ordering。Request-local external stream 使用对应 primitive 的顺序读取语义。

`std.http.stream` 返回 response handle，其 `body` 是 request-local external source stream；`std.http.sse`、LLM stream 等 native std / package API 也返回 request-local external source handle。调用方提前退出、当前 request timeout、外层 cancel 或 consumer `break` 时，runtime 必须 abort in-flight external request，或在无法底层取消时丢弃后续 external response 并走后台清理。

`std.file.createFromStream` 是 native host operation consumer：它只接受 `Stream<bytes>`，在当前 request 中顺序读取 chunk、写入不可变文件 staging，并在 source end 后提交文件。source error、request cancel 或 host operation error 必须取消相对侧，且不能留下已提交的部分文件。

External stream source error 映射为当前 lane 的 ordinary throw。若 server-stream operation 正在消费 external stream 并转换输出，inner stream error 会让当前 server-stream request error，除非用户源码捕获并收敛。

## 10. HTTP entry

HTTP entry 是 gateway-selected raw HTTP dispatch，不是 service-to-service API。

Router 根据 trusted / selected service id、version / build 和 loaded entry metadata 调用该 service 的 HTTP entry operation。Router 不做业务 route bind，不按 path 改写 request，不根据 content-type 自动 decode body。

外部 HTTP request 在 dispatch 前打包为标准 HTTP request envelope；method、url、path、query、headers 和 body 保持为业务可检查的数据。Query 和 headers 使用数组保留重复项和顺序。

Service 返回标准 HTTP response envelope；gateway 写回 status、headers 和 body。Raw streaming HTTP handler 返回 `Stream<std.http.HttpResponseStreamEvent>` 时，runtime 把 `start/chunk/end` event 转换成 `response.start/response.chunk/response.end` frame，gateway 按顺序写 socket。`start` 前的 runtime error 写 platform JSON error；`start` 后的 error 首轮按连接中断处理。Client disconnect 必须向 runtime 发送 cancel。

Typed HTTP route 是 compiler-generated wrapper，不是 router framework。Router 仍只选择 service/version/route 并发起 HTTP dispatch；wrapper 在 service runtime 内执行 `http.pre`、JSON body decode、handler 调用和 HTTP 200 JSON encode。越过 wrapper 的 `std.http.HttpError`、decode error 或平台错误通过 runtime `response.error` 映射为非 2xx platform error response。该 HTTP response body 固定为 JSON `{ "message": string, "detail": Json? }`，不暴露 internal `code` 或业务指定 status；平台策略选择 status，例如 body/schema decode 为 400、handler / `http.pre` 未捕获异常为 500、timeout 为 504、runtime/dependency unavailable 为 503。

HTTP status code 本身不是 throw；业务代码必须检查 status。HTTP entry 的可观测 target id 是 entry target；实际执行 deadline 由绑定 operation 的 service timeout / request deadline 决定。Host/path mapping 变化是 deploy / ingress 配置变化，不改变 service protocol identity。

## 11. WebSocket entry

WebSocket entry 只属于客户端直连的 API 层 service。下游业务 service 不拥有 Connection。

WebSocket 物理连接由 gateway / hub 维护。Connection 拥有 connection id、service id、状态、client session、actor binding、typed connection context、entry identity、绑定的 service protocol identity 和物理 socket 集合。

Connect operation 是一次 request frame，用于连接验证、actor binding 和 typed connection context 初始化。Receive / route dispatch 每次都创建独立 request frame。

Router 只做 entry routing 和 event envelope，不解释应用 eventName、业务 requestId、ack 格式或应用错误格式。Service 端回写或主动推送必须显式调用 `std.websocket` / client capability 这类 host operation；receive / route 返回值不是隐式 request-response message。

Business handler 不接收 raw socket id；它接收当前 actor 和 typed connection context。

WebSocket 连接按 entry identity 和绑定 operation 的 service protocol identity 路由。Schema-changing 发布产生新的 entry identity；既有 socket 继续绑定旧 entry identity，直到 drain 或断开。Runtime 不把旧应用消息投影到新 schema。

## 12. Effect metadata at runtime

Effect metadata 是 compiler / publisher / runtime 对调用影响的共享语义数据。Host operation 是 runtime 执行边界：一次调用离开纯 Skiff 求值，访问宿主能力、外部系统或跨 service transport。

Runtime 使用 effect metadata 解释调用属于 local read/write、external read/write 还是 telemetry / host write；解释 stable target id、timeout aggregation target、conflict-key、cancel-safety、commit point、cleanup action、idempotency、并发策略、测试替身和观测事件匹配。

Metadata 是语义承诺，不是日志注释。Metadata 缺失、target / conflict-key 无法静态确定、cancel-safety 不足或 provenance 不明时，compiler / runtime 必须保守拒绝不安全并发或重试。

在 `concurrent` sibling lane 中，read-only external effect 可以和其他 read-only external effect 并发；external write 默认不能和 sibling external effect 并发；同一 conflict-key 上的 read/write 或 write/write 不能位于不同 sibling lane；标记 `exclusive` 的 operation 在当前 request 内不能和任何 sibling external effect 并发。

只有 metadata 显式声明 concurrency safe、可比较 conflict-key 和足够 cancel-safety 时，runtime 才能允许更宽松并发。需要顺序执行多个冲突 effect 时，源码应把它们放到同一个 `serial { ... }` lane 或普通顺序代码中。

`config.require` / `config.optional` 读取当前 request frame 注入的 config view，是本地只读访问，不是外部 I/O。Array、Map、scalar receiver 方法只产生 local read/write effect，并按 mutable root provenance 参与并发检查。`std.json.encode` / `std.json.decode` 是 boundary codec helper，不访问外部系统。

`std.http.request`、`std.http.stream`、`std.http.sse`、service call、telemetry emit 和 WebSocket send 是 host operation 或跨 runtime operation，必须有 effect metadata。`std.websocket.sendJson<T>` 若只编码 JSON 并调用 sendText，则 host write 发生在 sendText；helper 本身可以作为 wrapper 暴露高层 target / timeoutTarget，但不能隐藏底层 cancel 和 trace 事实。

## 13. Current unsupported runtime capabilities

当前不支持：

- `detach`、后台 lane 或 request 结束后继续运行的 Skiff coroutine。
- 顶层 mutable 容器、语言级共享内存或跨 request heap handle。
- 本地 Skiff stream producer / coroutine；普通 Skiff 函数不能返回自己创建的可逃逸 `Stream<T>`。
- 双向 stream、用户 operation stream 参数、半关闭、reconnect / resume、持久化 stream cursor 或 `Stream<T, R>`。
- `Stream<T>` 作为用户 operation 参数、用户 record 字段、持久化字段、collection 元素或普通 public API type 字段。
- 语言级 snapshot / read view 表达式。
- request-scope component 或把 request-local 状态保存进 service revision singleton。
- queue、cron、async task 和 durable long-running workflow。
- 用户可观察的 runtime cancel inspection API。
- 函数体级 `concurrent` modifier。
- `concurrent` surface 内的普通控制语句、直接 throw/catch、`timeout(...)`、`with`、stream control、`spawn`、嵌套 `concurrent` 和 callback / anonymous function body。
- `return` 穿过 `concurrent` 边界；`break` / `continue` 穿过不在同一 lane 内的 loop 边界。
- Set surface、string indexing 和 heap cycle。
- 自动 retry 非幂等 operation。
- Router core 中的业务 host/path route bind、cookie/session/auth 解释或 WebSocket application protocol 解释。

这些限制是当前 runtime 合约的一部分，不应由 std wrapper、package wrapper 或 router 配置绕过。

未来支持这些能力时，必须显式定义其 request 生命周期、heap / root provenance、effect metadata、timeout / cancel 和边界 schema 规则，不能隐式复用当前 request frame 语义。
