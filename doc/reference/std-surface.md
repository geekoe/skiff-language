# Skiff Prelude And Std Surface Reference

本文负责：合并描述无需 import 的 prelude surface 与内建平台 `std` surface；覆盖核心类型、Date、平台错误、collection、scalar、bytes、JSON、config、`std.json`、`std.string`、`std.crypto`、`std.time`、HTTP helper、`std.log`、`std.websocket` 和测试替身边界。

本文不负责：完整语法、类型推断细节、service protocol identity、runtime transport 编码、manifest 字段表、测试发现和 runner 模式。

## 1. Roots And Visibility

prelude 类型和基础 receiver API 默认加载，不需要 `import`。它们进入全局 type namespace 或 method namespace。

`std` 是内建平台标准库 root，不是普通 package dependency。源码可以直接访问 `std.http.decodeJson`、`std.json.decode`、`std.crypto.sha256`、`std.log.info`、`std.websocket.sendTextToConnection` 等 surface；`import std` 可以保留为显式风格，但不是使用平台 std 的前提。

`std.<module>` 是平台 std 的模块化 helper path，不是 package id。模块函数一律走 `std.<module>.<name>` 路径，函数名不带模块前缀（例如 `std.http.json`，不是 `std.httpJson`）；HTTP 类型走 `std.http.*`，不拍平到 `std` root。命名规则与事实来源见 `../implementation/std-naming-and-source-of-truth.md`。`import std.http`、`import std.json` 和旧 `ext.*` 聚合 root 都不属于目标 surface。

根级语言内建（`bytes`、`Json`、`JsonObject`、`Stream<T>`、`config`）是例外：它们直接在 `std` root（或如 `config` 作为内建 value root），不进二级模块、不加前缀。

`config` 是内建 value root，不是 package，也不是 `std.config`。它暴露当前 service request frame 的只读 typed config view。

`root` 是当前 source set 的内建访问 root，用于当前 package / service 内跨文件访问；它不是标准库 API。

`skiff.run/std` 不是用户可声明的普通 package。`skiff.run/*` 中的业务/SDK package 仍是普通 package；LLM、provider SDK 或云厂商 package 不属于 prelude 或 `std`，应通过 manifest/package alias 后在源码中 import alias。

## 2. Prelude Core Types

基础类型包括 `string`、`number`、`integer`、`bool`、`null`、`Date`、`bytes`、`unknown`、`void` 和 `never`。

`bool` 是布尔类型唯一 canonical 拼写。`number` 是统一运行时数值类型。`integer` 是 finite safe integer refinement，运行时仍由 `number` 表示。

`integer` 可赋给 `number`；`number` 不能隐式赋给 `integer`，除非是可静态证明的整数 literal 或经过显式 safe integer 校验 API。

runtime prelude 类型包括 `Array<T>`、`Map<K,V>`、`Stream<T>`、`Config`、`Json`、`JsonObject`、
`Exception<E>`、`CatchResult<T,E>`、`SourceLocation`、`StackTrace` 和 `StackFrame`。

`Date` 表示 UTC instant。运行时内部表示为 epoch milliseconds；HTTP/API、service boundary、JSON schema 和 DB business JSON 统一以 RFC3339 UTC string 表达，例如 `2026-06-04T15:12:03.456Z`。可表示范围限定为 RFC3339 stable year `0000..9999`；超过范围的构造和 arithmetic 抛 `std.time.DecodeError`。leap second 输入不支持。

`Date` static surface 包括 `Date.now()`、`Date.fromEpochMilliseconds(ms)`、`Date.parse(value)` 和 `Date.requireParse(value)`。`parse` 对非法或越界文本返回 `null`；`requireParse` 抛 `std.time.DecodeError`。receiver surface 包括 `toEpochMilliseconds()`、`toISOString()`、`addMilliseconds(ms)`、`diffMilliseconds(other)`、`compare(other)`、`isBefore(other)` 和 `isAfter(other)`。

HTTP 类型不是 prelude，而是 `std.http.*` 模块类型，包括 `std.http.HttpHeader`、`std.http.HttpQueryParam`、`std.http.HttpRequest`、`std.http.HttpResponse`、`std.http.HttpClientRequest`、`std.http.HttpClientResponse`、`std.http.HttpClientStreamHandle`、`std.http.HttpSseEvent`、`std.http.HttpResponseStreamEvent` 和 `std.http.HttpError`（均不拍平到 `std` root，见 §11）。`std.websocket`不公开raw receive message类型；connect、send和平台拥有的outbound request/response surface见§13。Gateway/actor prelude不声明`ActorRef<T>`；每个actor声明的名义类型本身就是actor句柄类型。actor registry入口为`getOrCreate`、`replace`、`find`和`remove`。`ActorBinding`、`ClientSessionRef`和`ClientCapability`仍按各自surface定义；旧`std.actor.Actor<Id>`接口由显式actor声明及其id类型取代。

这些 prelude 名字不能被用户声明、import alias 或局部绑定 shadow。

## 3. Standard Platform Errors

标准平台错误都是带稳定名义identity的具体类型，不需要共同的marker interface。它们可被 `catch<E>` 捕获，
前提是错误发生在用户代码已经进入当前 Skiff request 后，或由用户代码发起的 std API / service call 产生。

当前 platform error surface 包括 `std.json.DecodeError`、`std.bytes.DecodeError`、`std.db.DecodeError`、
`std.db.ConflictError`、`std.file.FileError`、`std.number.DecodeError`、`std.time.DecodeError`、
`config.DecodeError`、`std.service.ProviderUnavailableError`、`std.service.ProtocolError`、
`std.service.InternalError`、`std.http.HttpError`、`std.websocket.WebSocketRequestError`、
`CancelError` 和 `TimeoutError`。

decode 类错误按所属模块命名，用于用户代码发起的 JSON、bytes、DB、file、number、time 和 config 转换失败。runtime 内部 decode / artifact / transport 不变量失败不暴露为用户可 catch 的 decode 类型。错误消息必须脱敏，不能包含 secret 或原始敏感值。

`std.db.ConflictError` 表示可重试的数据库写冲突或 transient transaction conflict。runtime 不会自动重放 transaction；调用方只应在确认整个重试边界不含外部副作用且具备幂等性时显式重试。该错误只暴露稳定、脱敏的 `target`、`message` 和 `retryable` 字段。

provider unavailable 类错误表示目标服务、网络连接、DNS、TLS 或 provider runtime 不可用。

protocol 类错误表示跨服务、HTTP/SSE 或 gateway/runtime 协议不匹配、无法恢复 identity 或 payload 与 lock / schema 不一致。

`std.websocket.WebSocketRequestError`表示Skiff主动发起的WebSocket request因connection、deadline、
cancel、platform protocol或peer显式error而失败；JSON编码和typed response decode失败仍使用
`std.json.DecodeError`。

`std.service.InternalError`用于隐藏不能安全保留原始类型的跨服务错误。用户错误为私有类型、不可name、
不满足`SchemaClosed`或实际编码失败时，runtime在该错误第一次越过service boundary时生成
`InternalError`，不发送原始type identity、字段或显示字符串。它是公开、schema-closed、可捕获且可继续跨
服务序列化的名义类型，包含固定脱敏`message`、`traceId`和唯一`errorId`。中间service未捕获时继续传播同一
错误值与关联identity，不重复包装。

gateway 在进入 service operation 之前发生的 HTTP / WebSocket decode error 不会被业务 service 捕获；它由 gateway 按外部协议返回。

## 4. Request-Local Control Flow Types

`Exception<E>` 是 request-local throw envelope，包含被抛出的值、source location 和 stack trace。每个错误
值都通过该envelope传播，所以`std.service.InternalError`与任意用户错误一样必定有栈；栈不是错误值的业务
字段。

`CatchResult<T,E>` 表达 `catch<E>` 的结果，逻辑上是 ok / err discriminator union：ok branch 携带值，err branch 携带 `Exception<E>`。

`Exception<E>` 和 `CatchResult<T,E>` 不是业务数据结构，不通过 boundary schema closure。它们不能出现在 service API、public contract type、跨服务 payload 或持久化 schema 中。

预期内业务失败应使用应用自定义命名 union 或 discriminator record union 表达，而不是返回 `Exception<E>`。

普通`throw`捕获当前request的source location与stack；同一request中的`rethrow`保留原envelope。跨service
传输只编码错误值和固定错误元数据，caller在调用点创建新的`Exception<E>`和当前这一跳的新栈，并加入脱敏的
remote-boundary frame。完整callee栈留在服务端telemetry/log，通过`traceId/errorId`关联。

## 5. Collections

`Array<T>` 和 `Map<K,V>` 是 mutable collection，携带 mutable root。`const` 只限制 binding，不让 collection deep immutable。

`Array.empty<T>()` 和 `Map.empty<K,V>()` 返回 fresh mutable root。literal 构造也返回 fresh mutable root。

`Array<T>` 提供长度读取、push、set、pop、clone、map 和 filter 等基础 surface。mutation API 写 receiver root；clone / map / filter 返回 fresh root。

`Array.map` 和 `Array.filter` 的 callback 是 lane-local non-escaping callback；callback effect 并入承载该 API 调用的 lane。

`Map<K,V>` 提供长度读取、keys、get、has、set、delete 和 clone 等基础 surface。set / delete 写 receiver root；keys / get / has / length 是 local read。

`Map<K,V>.keys() -> Array<K>` 返回 key 的 request-local 快照数组。调用后修改原 map 不改变该数组的元素集合；调用方可以像普通 `Array<K>` 一样修改该数组，且不会影响原 map。

`for key in map` 遍历 map key，等价于遍历 `map.keys()`。`for key, value in map` 遍历 map entry 快照，`key: K`、`value: V`。双绑定 `for` 不适用于 `Array<T>` 或 `Stream<T>`。

语言层只有一种 `Map<K,V>` surface，不区分 HashMap / TreeMap。`keys()` 和 map `for` 的遍历顺序是 canonical map key order，不是插入顺序。当前合法 map key 是 `string` 或 string representation，排序按 canonical string payload 的 UTF-8 字节序升序；未来如果扩展非 string key，必须先定义该 key 类型的 canonical map ordering。

当前不支持 `Set<T>` surface。未来如引入，需要单独定义 aliasing、wire encoding、canonical order 和 mutation API。

`{ ... }` 支持 target-typed map/json literal，但只在目标类型是 `Map<string,T>`、`JsonObject` 或 `Json` 的 object branch 时启用。无目标类型时不能自行推断为 map。

mutable collection 是 invariant；例如 `Map<string,string>` 不能整体赋给 `Map<string,Json>` 或 `JsonObject`。

## 6. Scalar And Bytes

`string` receiver surface 包括 length、contains、replaceAll、concat、lowercase、startsWith、endsWith 等基础操作。`string.join` 是 type namespace static helper。

string indexing 不属于当前 surface。字符处理必须走 string receiver 或 `std.string` API。

`+` 当前不定义 string 拼接语义；字符串拼接使用 `concat` 或 join helper。

`number` receiver surface 包括 floor、ceil、round。type namespace helper 包括 parse、isInteger、isSafeInteger 和 assertSafeInteger。

`number.parse` 接受有限普通十进制数值字符串；空字符串返回 `null`，非法数字、`NaN` 和无限值抛 `std.number.DecodeError`。

`number.assertSafeInteger` 对非 safe integer 抛 `std.number.DecodeError`，否则返回 `integer`。

`bytes` 是二进制值。base64、hex 和 utf8 是与外部文本协议互转的编码形式，不是业务代码中的独立 bytes 类型。

bytes surface 包括 concat、fromBase64、fromHex、fromUtf8、length、toBase64、toHex 和 toUtf8String。

`bytes.concat` 按顺序拼接 `Array<bytes>`；空数组返回空 bytes。

## 7. Json And JsonObject

`Json` 和 `JsonObject` 是 prelude 定义的递归 compiler-known 类型，用于表达裸 JSON 数据。

`Json` 的值域是 `null`、`bool`、`number`、`string`、`Array<Json>` 和 `JsonObject`。`JsonObject` 的 payload 语义等价于 `Map<string,Json>`。

`JsonObject` 不是普通透明 alias，也不要求用户 alias 支持递归。IR 和 schema 可保留它们作为 prelude type symbol / descriptor。

赋值到 `Json` 目标类型时，`null`、`bool`、`number`、`integer`、`string`、`Array<Json>` 和 `JsonObject` 都可直接进入。

`Map<string,Json>` 与 `JsonObject` 在 JSON 位置等价，但 mutable collection invariance 仍成立。

裸 `Json` / `JsonObject` 不携带用户 representation、union 或 map-key identity。把 representation 放进裸 JSON 需要显式 projection。

边界 payload decode 是 schema-directed；JSON 只是显式 codec 的一种 bytes/text 编码。需要恢复名义身份时不要先降入裸 JSON。

## 8. Config Root

`config*.yml` 合并后的配置视图通过顶层 `config` root 读取。service 代码和在该 service 中执行的 package 代码看到同一个配置视图。

`Config` 是 compiler-known prelude type，不是 `Map<string,Json>` alias。它只暴露 typed read，不暴露完整 object payload，也不提供 mutation API。

config path 是 dotted path，例如 `openai.apiKey`。每个 segment 必须匹配配置 path segment 规则；空 path 是编译错误。

path 必须是 string literal 或 compile-time const-foldable string。普通动态字符串调用不进入 publisher 的 config shape 收集。

`config.require<T>(path)` 表示 required path；缺失或 `null` 应导致 service activation 失败，运行时 shape 不匹配抛 `config.DecodeError`。

`config.optional<T>(path)` 表示 optional path；缺失或 `null` 返回 `null`，存在时必须匹配 `T`。空字符串是合法 string，不等价于缺失。

`require<T?>` 和 `optional<T?>` 非法；required / optional 由函数名表达，不由 nullable type argument 表达。

`config.has(path)` 只判断 path 是否存在且非 `null`，不替代 required config 声明。

当前 config 可解码基础目标包括非 nullable `string`、`number`、`bool`、`Json` 和 `JsonObject`。未来 record decode 需要补 schema closure 规则。

config read 只读取当前 request frame 使用的配置视图，不表示外部 I/O，不产生 external effect，也不改变 service identity。

package 代码读取 config 仍是当前 service config；package 不拥有自己的 secret namespace 或 per-service instance。

目标 surface 不提供读取整个 config object 的通用 accessor。旧 `values.object()` 不进入目标 surface。

## 9. std.json

`std.json` 提供 schema-directed JSON text codec，通过 `std.json.*` 访问。

encode 按 Skiff schema 把 value 写成标准 JSON 文本 string；无法编码的值抛 `std.json.DecodeError`。

decode 只接受 JSON 文本 string，并按目标 Skiff schema 恢复值；字段缺失、类型不匹配或 union discriminator 无法恢复时抛 `std.json.DecodeError`。

HTTP body 和 external payload 等 bytes 输入必须先显式转成 UTF-8 string；JSON codec 不隐式吞 transport bytes。

`decode<Json>` 覆盖旧 parse 场景，`encode<Json>` 覆盖旧 stringify / projection 场景。

## 10. std.string, std.crypto And std.time

`std.string` 放置不适合挂在基础 receiver 上的文本 helper，包括 split、ASCII digit 检查、query component encoding 和 path encoding。

split 返回 fresh array，separator 不能为空。`isAsciiDigits` 只对非空 ASCII 数字串返回 true。

URL percent-encoding helper 区分 query component 和 path；path encoding 保留 `/`。

`std.crypto` 提供少量 runtime-backed crypto / random helper，包括 HMAC-SHA1 base64、SHA-256、random token、标准 UUID 和不带连字符的 simple UUID。返回值是文本编码结果。

`randomToken`、`uuid` 和 `uuidSimple` 由 runtime 提供 request-local 调用；调用结果不应被当作 deterministic pure expression。

`std.time` 只承载 request-local time control API；wall-clock value 读取属于 `Date` surface。

`sleep(ms)` 只挂起当前 request，不创建 durable timer。`ms <= 0` 立即返回；单次等待最多 60 秒，超过上限按 60 秒处理。sleep 受当前 request timeout 和 cancel 约束。

含 `Date.now()` 的测试不应断言具体 instant；需要稳定值时使用 `Date.fromEpochMilliseconds(...)` 或运行时测试设施注入固定时间。

## 11. HTTP Std Surface

HTTP std surface 都在 `std.http.*` 模块下，属于内建 platform std，不通过普通 package resolver。函数名不带 `http` 前缀（`std.http.json`，不是 `std.httpJson`），类型走 `std.http.*`（`std.http.HttpRequest`），不拍平到 `std` root。命名规则见 `../implementation/std-naming-and-source-of-truth.md`。

更高层 SDK / wrapper package 应组合 std HTTP helpers，而不是各自定义 runtime native driver。

`std.http.HttpRequest` / `std.http.HttpResponse` 是 raw HTTP entry envelope。`std.http.HttpClientRequest` / `std.http.HttpClientResponse` / `std.http.HttpClientStreamHandle` / `std.http.HttpSseEvent` 是 outbound HTTP effect 的 request / response / stream handle / SSE event schema。

这些类型由`skiff.run/std` Package schema拥有。进入package/service boundary的schema-stable HTTP类型保留
其PackageSchemaTypeId；compiler不得把它们在operation位置展开为anonymous record，也不得在每个
ServiceContract中复制或重新生成service-owned类型。request-local handle仍不属于可远程传输schema。

HTTP headers 和 query params 使用数组保留重复项和顺序。

HTTP bodies 是 request-local bytes。JSON、text、form 和 multipart 都是显式 codec / helper 层，不由 router/runtime 按 content-type 自动 decode。

HTTP status code 本身不是 throw；调用方必须检查 response status。

DNS、连接失败、TLS、payload decode 或协议错误抛标准平台错误，例如 provider unavailable、protocol 或 decode error。

`std.http.HttpClientRequest.body: null` 或缺失表示空 body。`timeoutMs: null` 表示只受当前 request deadline、外层 `timeout` 和平台默认 operation timeout 约束。HTTP proxy 是 runtime/operator 本地资源，只能通过 runtime config 的 `http.egress.proxy` 配置；service 不能在 `std.http.HttpClientRequest` 中声明或覆盖 proxy。runtime 不读取环境代理配置。

`std.http.request` 返回完整 response body bytes。`std.http.stream` 返回一次性 HTTP stream handle，`status` / `headers` 同步可读，`body` 是 `Stream<bytes>`。`std.http.sse` 返回一次性 SSE event stream。

`std.http.json<T>` / `std.http.jsonWithHeaders<T>` 构造 JSON `std.http.HttpResponse`；`std.http.decodeJson<T>` 从 `std.http.HttpRequest.body` 做 schema-directed JSON decode。typed HTTP route wrapper 使用这些 helper，把 handler 正常返回统一编码为 HTTP 200 JSON。

`std.http.header` / `std.http.headers` 按大小写不敏感 header name 读取入口 request headers；`std.http.query` 按精确 query name 读取第一个 query value；`std.http.cookie` 从 `Cookie` header 中按精确 cookie name 读取值。

`std.http.errorResponse`、`std.http.noContent`、`std.http.methodNotAllowed` 和 `std.http.requireMethod` 是 raw route 的显式 response helper，不是 platform error channel。`std.http.forwardableHeaders` 过滤 hop-by-hop / connection response headers，`std.http.sseHeaders` 返回常用 SSE response headers。

`std.http.HttpError` 用于 HTTP handler 或 `http.pre` 主动抛出业务 HTTP failure，只携带 `message` 和可选
`detail`。越过 HTTP boundary 的 thrown failure 由平台选择 HTTP status，并写回固定 JSON body
`{ "message": string, "detail": Json? }`；业务代码不能通过 thrown error 指定 HTTP status 或 code。

`std.http.HttpResponseStreamEvent` 表达 raw HTTP streaming response：`start` 必须先于 `chunk`，`end` 后不能再 emit。`std.http.streamStart` / `std.http.streamChunk` / `std.http.streamEnd` 是构造该 stream event 的平台 helper。

调用方提前退出、外层 timeout 或 ancestor cancel 时，stream / sse 必须 abort in-flight HTTP request。

SSE helper 在 2xx 状态后输出完整 event；非 2xx 时按 body chunk 输出，供上层 package 读取有限错误体并脱敏。

effect metadata 默认按 method 推导：GET / HEAD 为 external read 且 idempotent，其他 method 为 external write 且 non-idempotent。

HTTP conflict-key 以 method 和 origin 为基础；origin 无法静态确定时为 opaque。stream / sse 的 cancel-safety 是 response-discardable。

## 12. std.log

`std.log` 是标准库日志 surface，不定义新的业务状态，也不应被业务逻辑依赖为可靠事件或审计记录。

日志级别 surface 包括 debug、info、warn 和 error。每次调用包含人类可读 message 和可选 `JsonObject` attrs。

attrs 是结构化 JSON object；runtime / exporter 可按 telemetry 配置丢弃、采样或脱敏。

effect metadata 是 telemetry write，target 对应具体 log level，cancel safety 是 fire-and-forget，business semantics 是 non-observable。

需要可靠业务事件时，应使用后续单独 event / queue API，而不是 `std.log.*`。

## 13. std.websocket

`std.websocket` 是 client-facing WebSocket connection 的标准库 surface。新连接入口由 service config
顶层 `websocket` 声明；path、domain 和 service selection 属于 ingress / router 配置。第一版每个service
最多一个entry，只拥有path与可选connect handler。

`WebSocketConnectRequest`、`WebSocketConnectResult`与connection policy是connect adapter使用的固定
platform types。它们可以通过std Package API被源码引用，但声明本身不生成ordinary PackageSchema，也
不能作为service-to-service payload。`service.yml`选择的connect handler从linked signature取得精确类型；
handler不要求出现在`api.yml`。

connect request包含connection id、url、query、headers、cookies、可选version、websocket entry id和
gateway entry identity。headers、query和cookies保留重复值。connect result是accept/reject
discriminator union；accept branch携带可选`businessIdentity`和`connectionPolicy`，reject branch携带
code与reason。

`std.websocket`不提供raw receive、client-initiated message handler、message selector或业务消息entry。
外部peer主动发起的业务请求必须走HTTP。没有匹配platform pending request的客户端text/binary data frame
以`1003`关闭；ping/pong/close由协议栈处理。

send target 分两套：`...ToConnection` 按单个 connection id 发送，`...ToBusinessIdentity` 按 business identity 发送（投递到该 business identity 当前的所有连接）。`std.websocket.sendTextToConnection` / `sendTextToBusinessIdentity` 发送 text frame，`std.websocket.sendBinaryToConnection` / `sendBinaryToBusinessIdentity` 发送 binary frame（不做 base64 编码）；这四个是 runtime host operation。

`std.websocket.sendJsonToConnection<T>` / `sendJsonToBusinessIdentity<T>` 是普通 std helper，使用 `std.json.encode<T>` 后分别委托对应的 text host operation，不是 host operation 本身。

WebSocket还提供一个通用的、由Skiff发起的request/response操作：

```skiff
type WebSocketRequestError {
  code: string,
  message: string,
  detail: Json?,
}

native function requestJsonToConnection<TRequest, TResponse>(
  connectionId: string,
  method: string,
  value: TRequest
) -> TResponse
```

平台把`value`编码进固定text JSON request envelope，并生成opaque `requestId`。外部peer可以异步、乱序
返回固定response envelope，但必须在原connection上原样回显该ID。Gateway/runtime只解析outer
`type/requestId/ok`与固定error shape；`method`、request payload和success payload保持opaque。匹配成功后
直接恢复等待中的调用，不创建service ingress request，也不调用用户handler。业务源码不接触transport
`requestId`。`method`必须是非空string；平台对method、encoded payload和pending数量实施固定上限。

Request payload按`std.json.encode<TRequest>`语义编码，success payload按
`std.json.decode<TResponse>`语义解码。请求值不可编码或success payload与`TResponse`不匹配时抛
`std.json.DecodeError`；这是调用点类型/peer application protocol错误，不会被伪装成transport
`WebSocketRequestError`，也不使Router按业务schema解释payload。

第一版wire精确为：

```json
{"type":"request","requestId":"<opaque>","method":"<method>","payload":null}
{"type":"response","requestId":"<opaque>","ok":true,"payload":null}
{"type":"response","requestId":"<opaque>","ok":false,
 "error":{"code":"<code>","message":"<message>","detail":null}}
{"type":"cancel","requestId":"<opaque>"}
```

JSON示例中的`payload`是任意JSON值，不限于`null`。Outer envelope字段严格；wrong-connection、
wrong-generation或未知id的response不能命中pending调用，并以协议错误`1002`关闭。平台保留有界短期
settled tombstone；与完成/取消竞态的晚到或重复response只命中tombstone并被丢弃，不能恢复调用。没有
pending request的普通data frame以`1003`关闭。第一版不支持binary request/response。

`requestJsonToConnection`只接受精确connection id，不提供business identity fan-out版本。多个socket不能
共同拥有一个unary response。调用受当前execution deadline与cancel约束；等待response时是真实
suspension point。connection关闭、deadline、cancel、协议错误或remote `ok:false`抛
`WebSocketRequestError`。Cancel/deadline先原子删除pending state，再在socket仍可写时best-effort发送
cancel envelope；晚到response不能恢复其它调用。Transport pairing不提供业务幂等、重试或exactly-once；
pending数量、payload大小和tombstone生命周期均受平台limit约束，达到上限时新request fail closed。
有持久副作用的协议仍保留自己的`toolCallId`、`attemptId`、`idempotencyKey`等业务identity。

WebSocket send effect是external write，conflict-key以connection id为基础，cancel safety是
response-discardable。`requestJsonToConnection`也是external write，但拥有response wait并被静态标记为
`maySuspend`；普通send保持non-suspending。

version 优先来自 `X-Skiff-Version`，WebSocket query 只作为兼容 fallback，表示选中的 service version，应与 service root version 对齐。

## 14. Stream Surface

`Stream<T>` 是 request-local 一次性顺序消费值，或服务 operation / ingress entry 的 server stream 返回类型。

作为 service operation 返回类型时，`T`必须通过Package schema closure。作为external ingress返回类型时，
`T`按该gateway entry的linked handler signature与external codec校验，不要求仅为ingress进入
PackageSchema；两种边界不能互相推导。

作为 std / package stream-producing API 返回值时，stream 是 external source handle；平台 std 也可以返回包含 stream 字段的 runtime-owned handle record，例如 `std.http.HttpClientStreamHandle.body`。这类 handle 只能在当前 request 内消费，不能持久化或作为业务协议 schema。

native host operation 可以声明特权 stream 参数，用来在当前 request 中消费 source stream。例如 `std.file.createFromStream(source: Stream<bytes>, ...)` 顺序读取字节 chunk 并创建不可变文件；该 stream 不会跨 service / gateway 边界传递。

stream 消费通过 `for event in stream` 顺序读取。end 正常退出；source error 映射为当前 lane 的 ordinary throw。

break、return、外层 timeout 或 ancestor cancel 必须向 source 传播 cancel。stream 完成、出错或取消后不能再次消费。

`emit` 是 server-stream producer 的 ordered external write，也是 backpressure point。它不能在 concurrent sibling lanes 中直接使用。

当前不支持用户 operation stream 参数、bidirectional stream、resume、cursor 或 `Stream<T,R>` 完成值。

## 15. Test Double Boundary

`std` host-backed API 必须能被 `skiff test` 或发布系统测试模式按 stable target id 替换。

测试替身按 target 和可选 conflict-key 匹配。典型 target 包括 `std.http.request`、`std.http.sse`、LLM stream、provider package operation 和 service operation target。

替身必须返回目标要求的schema-closed payload。替身可以抛任意语言允许的名义错误值；当它模拟service或
host boundary时，公开且schema-closed的错误保留原类型，私有、非closed或编码失败的错误对调用方表现为
`std.service.InternalError`，与真实boundary一致。

替身执行仍参与 effect summary；不能因为是 mock 就绕过 `concurrent` effect conflict 检查。

HTTP、SSE、WebSocket send、time、crypto/random 等 runtime-backed API 的替换应维持原 target id 和 effect category，使测试与生产的冲突检查一致。

double registry 在每个 test case 结束后清理，不能污染后续测试。request frame 结束后也不能保留 HTTP 替身状态。

生产 artifact 不包含 test-only source、test helper exports 或测试 config read metadata。

## 16. Surface Boundaries

prelude surface 是语言默认可见集合；`std` surface 是官方 package API；普通 provider SDK 和业务 package 是独立 package surface。

`Json` / `JsonObject` 适合动态 JSON 数据，不适合保留 Skiff 名义身份。跨服务 typed payload 应优先使用命名 schema。

platform errors 描述运行平台或协议层失败。业务可预期失败应进入 API 返回类型，不应依赖未捕获 throw 越过服务边界。

host-backed `std` API 必须发布 effect metadata，包括 target、conflict-key、cancel safety 和 stream / callback 行为。

新增 prelude 或 `std` surface 时，需要同时明确 namespace 归属、schema closure 能力、effect metadata、测试替身 target 和与 service boundary 的关系。
