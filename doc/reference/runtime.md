# Skiff Runtime Reference

本文负责：

- 定义 Skiff runtime 的稳定执行边界：gateway / router / service runtime 各自承担什么。
- 定义 request frame、heap value、writable place/`InOut`、concurrent lane、join、timeout、
  内部停止、stream 和错误选择的运行时语义。
- 定义 HTTP / WebSocket entry 与 service-to-service call 如何共享request执行机制，同时保持不同的
  identity与路由surface。
- 说明 effect metadata 在 runtime 中如何参与并发、timeout、内部停止、观测和测试替身。
- 列出当前明确不支持的 runtime 能力。

本文不负责：

- 不定义语法、类型推导、完整 schema closure、std / prelude API signature。
- 不复制 manifest、service config、transport envelope 或 std API 字段表。
- 不写实现迁移计划、artifact 文件格式、部署拓扑或数据库 adapter 细节。
- 不保留旧实现兼容语义；Skiff 尚未发布，本文按目标语义收敛。

## 1. Runtime boundary

Skiff runtime 是一组边界职责，不是用户源码可访问的全局对象。

Gateway adapter 负责外部协议适配：接收 HTTP、HTTP stream / SSE、WebSocket 等入口，维护外部连接，执行协议层 decode / encode，把外部入口转换成 router 可路由的 typed dispatch，并把 unary response、stream chunk、stream end 或 error 编码回外部协议。Gateway 不执行用户 Skiff 代码，不拥有 Skiff call stack 或 request heap。

Hub / router 是独立于用户 service runtime 的平台基础设施。它负责Runtime session注册、
release pointer解析、loaded/lazy-load capability选择、service protocol identity与gateway entry identity
各自的匹配、client session / actor binding / WebSocket Connection索引、in-flight request /
stream配对、internal stop hint / drain路由和容量门禁。Router core 不使用HTTP Host、cookie、
session、应用WebSocket eventName或业务requestId选择service。它先严格解析ingress注入的
`x-skiff-service`/`x-skiff-version`选择唯一精确deployment，再只在该deployment内解释已声明的
method/path。专用WebSocket request broker拥有编码无关的transport request identity与pending
state；第一版JSON-RPC 2.0 text adapter解释其控制字段，但不能把transport `id`投影成
业务字段或据此选择用户handler。

Service runtime 是执行用户 Skiff 代码的边界。它按exact deployment `buildId`加载、验证并缓存
immutable `DeploymentExecutionImage`，为每次dispatch创建request frame，解码payload，调用
implementation method或entry handler，执行表达式、函数、collection mutation、`concurrent`、
`timeout(...)`、`emit`和cleanup，并编码response/chunk/error。

Service runtime不维护外部WebSocket物理socket生命周期。WebSocket Connection属于gateway / hub；
connect handler dispatch是进入service runtime的一次request。Skiff主动发出的WebSocket request在原
request frame内挂起；peer response由平台broker关联后恢复该frame，不创建新的service dispatch。
Peer向`websocket.yml.jsonRpc`已声明method发起的request则创建独立gateway request frame。Raw frame
receive属于平台transport阶段，不是用户handler。

生产 runtime 的 artifact 信任边界是平台 build service。Runtime 不把开发者本地编译产物视为线上发布权威；线上 artifact 必须由平台 build service 产生、签名并记录 provenance。

## 2. Dispatch and request frame

以下事件创建request frame：unary service API call、server-stream service API call、HTTP entry
dispatch、WebSocket connect dispatch、declared WebSocket JSON-RPC method dispatch，以及测试runner构造的
等价dispatch。单纯收取raw WebSocket frame本身不执行用户代码；匹配平台pending request的peer response只
恢复已有request frame。

Gateway / router 可以维护 entry envelope、routing context、transport state、Connection 和 stream pairing state，但这些不是 Skiff request frame。

Request frame 包含 request 参数、request context、deadline、trace、runtime内部停止状态、Skiff call
frames、slot values、request-local heap、writable-place/`InOut`loan state、exception envelope、
`concurrent` lane state、join state、server-stream sink 或 external stream source handle，以及 request 内
创建的resource handle。

Request frame 不包含外部 WebSocket 连接生命周期、跨 request 业务状态、持久化数据库状态或
ambient/current release state。

Unary request 在 response end、response error、timeout 或runtime内部停止后结束。Server-stream request 在 stream end、stream error、timeout 或runtime内部停止后结束。Server-stream request 仍是一段有限 request 生命周期，不是 WebSocket connection，也不是后台任务。

Request 结束后，request heap、call frames、slot values、lane state、exception envelope 和 request-local stream / resource handle 全部清理。Heap handle、`Exception<E>`、`CatchResult<T, E>` 和 request-local stream 不能逃逸到 request 结束之后。

### Tail-call execution and recursive stack safety

Skiff保证一类明确的本地尾调用不随递归次数增加程序调用深度或宿主native stack。一个调用只有同时满足以下
条件才属于这项保证：

- 它是显式`return`的完整返回表达式，例如`return next(value)`；位于普通nested block、`if`、statement
  `match`或普通loop body中的显式`return`适用同一规则。
- linked target是当前program中的exact executable。它可以是direct self、mutual recursion、同一Package
  跨source module的function或静态解析的impl method；不要求callee与caller是同一个symbol。
- caller与callee在deployment link形成的concrete generic specialization下return plan canonical-equivalent，因此删除caller frame
  不会跳过representation、nominal identity、union branch或container carrier materialization。
- 调用点没有仍需在callee完成后运行的catch、timeout、transaction、concurrent join、stream cleanup或其它
  lexical continuation。

括号不改变上述位置。下面的调用不是这项保证中的尾调用：

- `return 1 + recurse()`、`return wrap(recurse())`中的内层调用、作为另一个调用参数的调用，以及
  constructor、representation wrap、interface box或其它结果转换内的调用；
- `return catch<E>(recurse())`中的try expression，因为caller仍需生成`CatchResult`；
- `timeout(...)`、DB transaction/lease、`concurrent` lane/value或尚未完成stream consumer cleanup中的调用；
- deferred stream producer、service call、Actor dispatch、callback capability、native/builtin或尚未解析为
  exact local executable的interface dispatch；
- `dispatch`与`emit`；前者提交持久 detached task，后者的参数求值不是callable return。

`maySuspend`本身不是尾调用障碍。Exact local callee可以在同一个request、execution scope、heap和Actor
execution frame内挂起并恢复；只有真实pending work适用既有suspension规则。Actor method内对local helper的
exact executable调用可以是尾调用，但Actor dispatch本身不能越过deployment/instance owner或arena fence
边界。

尾转移仍按普通调用逐次计入instruction budget，并在每个callee function entry执行deadline、internal stop
和budget checkpoint。无限尾递归因此必须被既有execution budget有界终止，不能因为不增长stack而逃逸。
参数继续在caller环境中按源码顺序求值且只求值一次；转移后callee复用同一request heap，最终结果只按共同的
return plan物化一次。

异常诊断不为每个已消除的尾调用保留一帧。Runtime保留真实non-tail caller前缀，并把当前tail-transfer site
用于该转移本身的错误归因；更早、已消除的tail site不进入最终stack trace。这样错误栈与执行空间都保持有界，
`throw`/`rethrow`的payload、catch identity、`traceId`和`errorId`仍遵守本文件的错误规则。

不满足尾调用条件的普通本地调用保持现有求值顺序和return materialization。只要实现仍以nested native
future执行这些调用，runtime就必须在安全native stack边界之前以结构化
`ResourceLimitExceeded(resource = "programCallDepth")`失败；该保护只统计真实active non-tail program
frames，不能用于限制或模拟合格尾递归。当前保护值是runtime safety implementation detail，不是语言关键字、
manifest字段或用户可配置budget；错误payload报告本次实际执行的limit。

内部owner、trampoline与验证契约见
[`../architecture/tail-call-execution.md`](../architecture/tail-call-execution.md)。该architecture文档不另行
定义tail position或用户可观察递归语义。

## 3. Runtime transport model

Runtime 内部transport分别表达service operation dispatch与typed gateway entry dispatch，不把raw
socket、raw WebSocket frame、raw SSE event或宿主语言HTTP object暴露给service runtime。External ingress
使用`GatewayEntryIdentity`和精确handler target，不伪造`ContractOperationId`。

HTTP与WebSocket external ingress使用两阶段路由。Router外部ingress可以按Host等平台规则注入可信
`x-skiff-service`与`x-skiff-version`；Router严格解析后，经release pointer取得唯一精确deployment
`buildId`，再在该deployment内按HTTP `(protocol, method, path)`或WebSocket `(protocol, path)`选择gateway
entry。Host仍保留在HTTP request envelope供业务检查，但不参与Router lookup。直接向Router提供这两个
header是Skiff production boundary；Skiff不在Router内重做Host映射。

Router到Runtime的request frame必须携带精确deployment `buildId`与gateway entry identity。Runtime只接受
与该buildId懒加载并验证完成的`DeploymentExecutionImage`逐项匹配的tuple，不从Host、path、latest pointer或
ambient registration重新推断。不同service可以共享相同method/path；同一deployment重复selector、缺失/
非法/歧义header、pointer记录不兼容和跨build substitution都fail closed。

该路由模型不再拥有active assembly、activation admission或generation frame。旧Host-bearing route、裸全局
ingress key、assembly/config-snapshot ref和旧frame不兼容读取。可选`ReleaseBundle`只是离线发布清单，
不进入request transport。

Unary dispatch 最多产生一个 final response：成功时 response end 携带 payload，失败时 response error 携带 runtime error envelope；unary dispatch 不能产生 stream chunk。

Server-stream dispatch 产生有序 chunk 序列，最后产生一个 end 或 error。普通 service stream 中，每个 `emit` 对应一个 response chunk；normal exit 对应无 payload 的 stream end；throw、remote error、decode error或timeout对应 stream error。Consumer已经离开时runtime只结束内部producer并丢弃晚到frame，不再构造公开cancel error。HTTP response stream 额外先发送 `response.start`，再发送 `response.chunk`，最后发送空 `response.end`。Stream end 后没有额外 final aggregate response。

Payload 在进入 service runtime 前按 expected schema decode，离开 service runtime 时按 response schema encode。JSON 只是显式 codec 的一种，runtime boundary 不把裸 JSON DOM 当作默认 payload 语义。

Router operator配置必须给出正safe integer `http.maxRequestBytes`与`http.maxResponseBytes`，没有per-service
override或隐藏默认值。Router在读取完整request body前限制前者；Runtime按bootstrap下发的后者尽早停止
过大response，Router在external boundary再次校验。HTTP stream按同一response生命周期累计，不能通过拆
chunk绕过；WebSocket frame使用自己的独立limit。

Skiff不提供公开的“取消请求”能力。Runtime内部仍有停止信号，用于deadline、并发loser、consumer提前结束stream、connection owner结束和drain等生命周期事件。该信号只停止不再需要的计算并隔离晚到结果；internal transport可以发送幂等、best-effort的stop hint，但hint不是业务协议、可靠撤销或公开terminal。断线时可能已经没有可写response，不能为了“明确取消”再制造普通错误。

默认不自动 retry。只有 effect metadata 和 operation schema 明确声明幂等、可比较 target / conflict-key，并由平台策略允许时，router 才能重试。

DeploymentExecutionImage对Package diamond保留每条真实dependency edge，但同一精确build沿direct与
transitive路径到达时只建立一个code slot、一份Package-scoped `ConfigView`和一个DB metadata owner。Package alias
不参与这些identity。同一Package ID解析到不同build、logical collection identity缺失/重复或system
physical-name encoding collision都fail closed。

每个service只获得一个数据库。数据库identity由operator选择的受信Mongo endpoint/storage domain、
profile与serviceId共同定界；语言模型不引入`platformId`。Package不声明database state requirement，
service/profile也不配置namespace；只有当前deployment package closure包含DB metadata时Runtime才按需提供
service DB handle。同一service的Package共享该数据库，但每次DB operation仍以精确PackageArtifact/File
IR/type identity选择schema和collection。跨service DB访问禁止。
Physical collection由stable`(packageId, declared logical collection identity)`确定性系统编码；不同
Package使用相同裸collection名字不会共享storage。Package dependency、requirement、binding及配置文件都
没有author-provided collection-name mapping。

发布工具把校验后的typed config graph冻结成ServiceDeployment-owned protected payload，并让deployment
buildId提交其不可替换opaque ref。Runtime按buildId load image时只从该payload为精确Package build构造只读
`ConfigView`；不得读取ambient/latest配置，也不存在独立`RuntimeConfigSnapshot`、config generation或
cold-recovery re-pairing。

### Service call 的 deployment owner

同一进程中的service call仍然跨service boundary。每次调用开始时，boundary resolver按依赖中冻结的
`serviceId + exact version + expected protocol identity`解析当前release pointer，取得并pin精确provider
build。进入provider时，Runtime一次性切换到该build的deployment owner：

- provider看到自己的Package配置、service DB、file、actor、dispatch、WebSocket、telemetry及service
  dependency；
- deadline、内部停止、time source、request lifecycle、trace/error、runtime request identity、
  stream lifecycle、测试effect/case capability及heap上限仍属于原request；
- provider使用fresh request heap，参数、返回、错误、callback payload和stream item按ServiceContract
  materialize，不能直接共享caller heap引用；
- caller call frame、writable place/`InOut` loan和actor execution frame不进入provider；内部actor句柄已有显式owner，
  不因service owner切换而改写；
- Package静态资源始终随当前执行callable的Package projection选择，不随deployment context复制。

pointer缺失、protocol不匹配、provider build无法加载或目标owner不可用时，调用在provider执行前失败，
不得按display name、ambient service或其它version猜测。provider image已在同一进程时，scheduler可以用
child fiber同步执行；否则走runtime transport。两条路径都使用fresh provider heap和相同boundary plan，
只有解析、transport或child执行真实等待时才产生Pending。普通continuation不切换owner；只有进入service
provider及调用request-scope callback capability时发生这种原子切换。

若service返回stream，stream继续pin创建它的provider build；后续item、callback和terminal都在原owner中
完成，直到stream end/error/drop才释放。Release pointer更新只影响后续新调用，不迁移既有stream或在途
invocation。

### Runtime连接并发门禁

初期并发门禁只由Router静态配置，不做CPU、内存、数据库或其他动态资源admission。`router.yml`必须在
现有`runtime`段提供正的安全整数：

```yaml
runtime:
  path: /runtime
  port: 4001
  maxConcurrency: 128
```

`runtime.maxConcurrency`是每条Runtime WebSocket连接统一的普通request上限。Router把已经交给该连接、
尚未terminal的HTTP unary/stream、WebSocket connect、WebSocket JSON-RPC、service call、
package-test root以及leased task attempt对应的active request都计入同一个pending计数；
scheduled / ready backlog不计入任何Runtime connection。`response.end`、
`response.error`、cancel/timeout收束或连接断开会释放计数；Actor与其他control frame不计入。

未绑定的请求可以选择另一条仍有容量的合格Runtime连接；已经钉死连接或deployment build的请求不能为了绕过
容量而迁移。目标连接已满且没有其他合法选择时，Router立即返回overload，不排队、不重试，也不创建
新的pending。多条Runtime连接分别应用同一个配置值，各自独立计数。

Service源码、`service.yml`、任何`config*`文件、ServiceDeployment和可选release bundle都不能声明、复制或
覆盖并发上限。Service profile的整个`lifecycle`配置面非法；旧`maxConcurrency`和`idleTimeoutMs`都已删除。
并发门禁不属于service ABI、artifact identity或Runtime bootstrap wire。

## 4. Runtime identities

Runtime 依赖稳定 identity 做路由、drain、观测和测试替身匹配。

Service protocol identity 描述 service-to-service API 的公开协议。API 参数 / 返回类型、operation 集合和 public schema closure 的规范化 schema 变化会改变它。跨 service call 的寻址坐标是 service id + version：caller 在依赖约束里声明被调 service 的 id 和 version，router 在请求时把 (service id, version) 解析为该 version 当前的 build 并路由到对应实例。发布时冻结的 build id 与 protocol identity 不是 release selector，而是边界兼容性 witness——dispatch 时 router 校验所解析当前 build 的 protocol identity 是否满足 caller 冻结的期望，不满足则以明确错误失败，绝不静默路由到不兼容的 build。

Gateway entry identity只描述external ingress的公开协议面。已冻结的HTTP identity覆盖entry/protocol
kind、unary/stream mode、external request/response shape、影响wire的标准source选择和公开external
error projection。WebSocket connect identity覆盖connect request/result shape、允许frame类别、
JSON-RPC profile版本与connection policy shape；每个declared JSON-RPC method另有entry identity，覆盖
params/result shape、adapter sources和固定external error projection。外部method仍是selector。
Selector、handler/pre/guard callable、
目标参数名、Package build、deployment policy、内部connection-context nominal identity/codec及完整
adapter execution plan不进入该identity；只替换实现而外部协议不变时只改变deployment `buildId`。
任何gateway entry变化都不改变service protocol identity。

Stable target id描述执行类别，而不是资源实例。Service operation、HTTP entry、WebSocket connect、
connection close平台事件、WebSocket send/request等std host operation和Package wrapper都必须能映射到
stable target。Target用于
effect metadata、timeout source、trace、日志、指标、测试替身和错误聚合。

HTTP dispatch使用gateway entry identity做匹配与观测。WebSocket Connection必须绑定connection protocol
identity与exact deployment build；outbound response必须精确匹配原connection、socket generation与
transport request id，inbound method也必须在同一pinned build中解析gateway entry。它不绑定或冒充
service-call protocol operation；schema-changing发布后，旧Connection继续使用其已pin build，直到drain
或断开。

## 5. Request heap and values

`null`、`bool`、`number` / `integer` 表现为值语义。`string` 和 `bytes` 对用户表现为不可变值；实现可以选择 inline、shared buffer 或 heap 优化，但不得暴露可变 alias。

`Array<T>`、`Map<K,V>`、`JsonObject`和record/object是aggregate value。赋值、普通参数传递、
返回和container store都产生当时内容的logical snapshot；后续从任一writable binding修改时，
不影响其它snapshot。实现不必deep copy：可以move unique backing，或通过share transition与
path copy-on-write保留语义。用户程序不能观察physical handle/backing identity。

局部`final`初始化与普通parameter binding都取得logical snapshot，并且是immutable runtime binding；从它们
派生的field/index path不可写。Aggregate writable access-path root只来自三类：局部`var`、当前有效的
`InOut` loan，以及Actor method中允许直接写入的`self.field` member/index path。局部`var`可重绑；直接
`self.field` path mutation写Actor shared state，把该field先读入普通local或传给普通parameter则只得到
snapshot。Actor method的DB transaction body仍禁止直接或经callee写Actor field，包括以该field path为
receiver的collection mutation。

顶层`const`位于immutable `ConstantHeap`，是request-independent且deeply frozen的值；把const aggregate放入
`var`后，第一次写入会创建request-owned COW node，不改变constant graph。

普通function/Package的参数与返回值都是logical snapshot，不携带caller-writable origin；callee不会因收到
Map、Array或其它aggregate就获得caller-writable reference。ServiceContract与gateway boundary还会把值
materialize到接收方heap，同样不能传递writable origin或`InOut` loan。只有静态解析到exact
Package-local/package-direct concrete callable的显式`inout`参数是短期exclusive write-through loan；callee
target必须经verifier证明`NoPending`（`maySuspend = false`）。`inout`只进入Package Local ABI，不得进入
service/gateway/interface/callback/Actor external/host effect/recoverable boundary；ordinary throw不回滚已执行
写入。

### Bracket/index execution

Postfix `object[index]` 是 strict collection access。Ordinary read 始终先求值 `object`，再求值
`index`，两者各恰好一次：

| receiver | selector | result | failure |
| --- | --- | --- | --- |
| `Array<T>` | `integer` | `T` | `index < 0` 或 `index >= length` 抛 `std.collection.IndexOutOfBoundsError { index, length }` |
| `Map<K,V>` | 精确 `K` | `V` | missing 抛 `std.collection.MissingKeyError { container: "Map" }` |
| `JsonObject` | `string` | `Json` | missing 抛 `std.collection.MissingKeyError { container: "JsonObject" }` |

`string`、record/object 和未收窄 `Json` 不进入该 runtime path。`Map<K,V>.get(key: K) -> V?`
仍是独立 receiver API；missing 返回 `null`，不执行 strict bracket throw。

Read result 按 linked image 中该精确结果类型的 lifecycle / `ValueTransferPlan` 产生 logical
snapshot，不返回 container 内部的 writable alias。Snapshot-capable aggregate 可做 O(1) share transition
并在首次写入时 COW；move-only/affine 结果或缺失 linked lifecycle proof 的 access 必须由
source checker/verifier 拒绝，VM 不得用 raw handle copy 充当 read。

Indexed assignment 的顺序固定为：

1. writable root 只解析一次；每个动态 selector 按 path 从外到内各求值一次，并解析、
   检查对应 path segment；
2. 所有 intermediate Array element / Map key / JsonObject key 都必须已存在。Terminal Array
   selector 也必须已在界内，只能 replace；terminal Map/JsonObject selector 允许 missing，并在
   commit 时 upsert；
3. 整条 path 解析成功后，RHS 恰好求值一次；
4. runtime 按 linked lifecycle 准备 COW/transfer，然后执行一次原子 logical store。

任一 selector、path check、RHS 或 store preparation 失败都不会暴露部分 container mutation。这个
atomicity 不回滚 selector 或 RHS 本身已执行的外部副作用。`Array<T>.set(index: integer,
item: T)` 与 terminal Array assignment 使用同一 replace-only/越界规则；
`Map<K,V>.set(key: K, value: V)` 与 terminal Map assignment 使用同一 upsert 规则。

含 `inout` 的 local/package-direct call 按源码 argument 顺序求值，每个 ordinary argument、root 和
index selector 都只求值一次。所有 `inout` path 的 intermediate 与 terminal segment 都必须已
存在；Map/JsonObject terminal key 在此不使用 assignment upsert。只有在全部 argument/selector
求值与全部 path check 成功后，runtime 才原子取得整组 exclusive loan；失败时无部分
loan 且 callee 不进入。Callee 进入后的写入是 write-through；ordinary throw 不回滚已执行写入。

每个可失败 bracket/path segment 都必须保留自己的 source attribution。生成的 collection error
使用失败 segment 的 source site 创建 exception envelope；`Array.set` 越界使用该 receiver call
site；missing key 不进入 payload、message、trace 或
telemetry。该分类只用于 source-visible collection access，不把 artifact/VM 内部 index 失败映射成可捕获
collection error。

每个request owner（包括service boundary创建的provider owner）都创建fresh request-local managed heap。
Request参数、DB/HTTP/service/external边界物化值、literal、COW node、需要heap表达的nominal wrapper与
request-local resource handle都属于对应heap。Collector对每个request heap都可用，但只在allocation
pressure触发；“长/短请求”不是语义分类。低分配request可在从未运行collection的情况下结束，最后整体
释放heap。

Opaque resource、transaction、stream source与其它native handle是identity-bearing capability，不因aggregate
value semantics自动获得copy/COW。它们的copyability、owner、close和escape规则由各自surface明确
定义。Heap handle本身是request-local id，不是跨request、跨artifact或跨service的ABI。
`Stream<T>`明确是affine one-shot handle：传参/返回只能转移consumer ownership，不能普通copy、放进多个lane
或开始第二次迭代；normal end、error、break、return、timeout与request stop都恰好一次释放endpoint并传播
best-effort source stop。

目标value graph不支持用户可见cycle。会形成cycle的mutation、materialize或wire payload必须
fail closed。Nested mutation沿writable path检查每个node的share state，只复制需要分离的path；
不允许只检查最外层collection而泄漏嵌套alias。

`Map<K,V>.keys()` 返回一个`Array<K>`快照值。返回值放入`var`后修改不影响原 map；
调用后修改原 map 也不改变该数组的元素集合。

map `for` 循环在循环开始时读取快照。`for key in map` 读取 key 快照；循环期间对 map 执行 `set` / `delete` 不改变本轮将访问的 key 集合。`for key, value in map` 读取 entry 快照；循环期间对 map 执行 `set` / `delete` 不改变本轮 key/value 对，若某个尚未访问的 key 被重新 `set`，后续迭代的 `value` 仍是循环开始时的 value。

map key 快照顺序是 canonical map key order，不是插入顺序。当前合法 map key 是 `string` 或 string representation，排序按 canonical string payload 的 UTF-8 字节序升序；未来如果扩展非 string key，runtime 必须先为该 key 类型定义 canonical map ordering。

string representation map key 在 request heap 中按 erased string payload 保存。`Map<UserId,V>.keys()` 的运行时数组元素仍是 string payload；静态类型和 boundary 编码通过 expected `Array<UserId>` schema 保留 `UserId` 身份。运行时不得把该结果重新推断成 untyped `Array<string>`。

## 6. Concurrent lane model

`concurrent` / `serial` 在 v1 暂不支持：编译器在语义阶段拒绝 `concurrent` 语句、`concurrent value` 表达式与 `serial`（编译期报错，如 `concurrent is not supported in v1`）。本节其余内容保留为未来恢复该特性时的语义目标。

`concurrent` 是结构化并发语义。无依赖且通过 effect / mutation 检查的 sibling lane 必须能在 async host / service await 边界真实重叠执行；实现可以重排无依赖 lane，但不能把 `concurrent` 降级为纯串行执行。用户可见 join、错误选择和 mutation 规则必须确定。

`concurrent { ... }` 只把被修饰 block 的第一层直属项划分为 lane：直属 statement 是一个 lane，直属 `serial { ... }` 整体是一个 lane，`concurrent value { ... }` 的 tail expression 是保留 `tail` kind 的普通 synthetic lane。当前 `concurrent` surface 是受限 lane list，不是普通 block；`if`、`match`、loop、`with`、`timeout`、普通 `value` block、`return`、`break`、`continue`、直接 `throw` / `rethrow`、`catch`、`emit`、`dispatch`、嵌套 `serial`、嵌套 `concurrent` 和 callback / anonymous function body 在该 surface 内非法，包括在直属 `serial { ... }` 内非法。被调用函数内部仍可包含普通控制流；lane 只观察其normal return、throw、timeout或内部停止结果。

`concurrent` block自身是词法作用域，但只有直属`final` declaration lane的结果能被后续
sibling lane读取。后续lane只能读取source position严格在前的sibling-visible `final`；读取
后方声明是forward reference。`var`在`concurrent` surface直属位置非法；嵌套block与`serial`
内声明不跨sibling可见。

Compiler为每个`concurrent` block建立lane DAG。Lane B读取lane A的sibling-visible `final`，则A
必须先于B完成。传入B的aggregate是A结果的logical snapshot。Tail lane依赖它读取的前序
`final`，也隐式依赖source position在它之前的所有lane normal exit。

Sibling lane禁止写入从`concurrent`外层捕获的`var`或发起`inout`调用，即使静态路径是
不同字段也一样。Lane-local `var`可在该lane内mutation。普通aggregate snapshot不携带共享的
caller-writable reference；identity-bearing resource仍按其effect/conflict-key检查跨lane冲突。`serial { ... }`
只收束顺序逻辑，不绕过外层writable-place限制。

`concurrent` block normal completion 前，所有已启动 lane 都必须 normal exit。某个 lane error、外层有效 timeout 或 ancestor内部停止确定获胜事件后，尚未启动的 lane 不再启动，正在运行的 lane 收到结构化停止信号，被停止lane后续产生的值、错误和 Skiff 可见写入被丢弃。已提交外部副作用不回滚，只能依赖effect分类、幂等、日志和补偿策略治理。

当前不定义 detach lane 或未归属后台 lane。

## 7. Error selection

Block 的退出结果属于普通完成、正常控制退出、错误退出、timeout退出或runtime内部停止。`return` / `break` / `continue` 是正常控制退出；`throw` / `rethrow` 是错误退出；block-level `timeout(...)` 产生 timeout退出。内部停止不是用户可触发或捕获的普通异常。

同一个 `concurrent` block 中，用户可见错误选择必须确定：外层 `timeout(...)` 或 request deadline 形成的有效 timeout 优先于 lane error；多个 sibling lane error 同时成为候选时，源码位置靠前的直属 lane 获胜；嵌套 timeout 同时到达时，只最外层 timeout 产生可观察事件。当前 `timeout(...)` 不能出现在 `concurrent` surface 内。

外部 API operation timeout 是该 API 所在 lane 的 lane error，不享受 block-level timeout 的最高优先级。用户手工抛出的 `TimeoutError` 也是普通 lane error。

获胜事件确定后，外层 `catch` 只能捕获该事件对应的 exception envelope。其他 lane 后续错误只能进入平台日志 / trace。

服务 API 不在函数签名或operation contract上声明业务 `throws`，也不发布推导出的throw set。预期内业务失败
应收敛为返回类型，例如 named union 或 discriminator record union。任意用户名义 `type` 都可在当前request
内被抛出；这不要求该类型可序列化。

未捕获错误越过service boundary时，runtime首先检查实际类型自己的Package owner。只有该类型在owner
`api.yml`中显式公开、满足`SchemaClosed`且成功编码时，response error才携带其精确
`PackageSchemaTypeId`和payload；caller链接同一identity后恢复原名义值。私有、不可name、非closed或编码失败
的错误不发送原type identity、字段或显示字符串，统一替换为可序列化的
`std.service.InternalError`。错误可能由throwing service的dependency package声明，判断始终使用类型自己的
owner。

Service-to-service 和 gateway-to-runtime 的 response error 在 caller 侧恢复为普通 throw envelope；
`catch<E>` 不区分本地throw与远程throw。`std.service.InternalError`是普通可捕获错误；中间service未捕获
时直接继续传播同一个错误payload和`traceId/errorId`，不增加包装类型。

每个 `throw` 都生成包含source location和stack trace的request-local `Exception<E>`；同一request中的
`rethrow`保留该envelope。跨service只传输错误payload与固定envelope，不序列化callee的
`Exception<E>`。caller在service call site创建新的本地exception stack，并附加脱敏remote-boundary frame；
因此A的错误经未处理的B继续传播后，B的caller得到相同错误值，但得到自己这一跳的新栈。各service完整本地栈
只进入受限telemetry/log，并通过`traceId/errorId`关联，不能把私有源码路径或原始私有错误字段暴露给caller。
InProcessBoundary与remote binding必须遵守同一规则。

Ingress decode在进入external handler前失败时，业务代码尚未运行，不能被业务`catch`捕获。

## 8. Timeout and internal stop

每次 request 在一个有效 deadline 下执行。`timeout(...)` block 只能收紧当前代码块和其中远程 / host 调用的有效 deadline。

一次远程调用或host operation的可见deadline，是调用点current execution deadline与该operation已有的
primitive operation timeout中最先到达者；current execution deadline已经包含caller request deadline
和外层`timeout(...)`的收紧。第一版service call不另外定义consumer dependency timeout或callee
operation timeout；需要更短调用预算时由caller显式使用`timeout(...)`。Service业务配置不拥有
deployment timeout。

Router `requestTimeoutMs`是external business request的平台上限，不是所有Router工作的通用timeout。
Deployment image lazy-load/preload和WebSocket connection drain使用各自operator-owned lifecycle budget；
它们不能继承或覆盖`requestTimeoutMs`或用户代码`timeout(...)`。反过来，load/drain budget也不能改变普通
dispatch deadline。Skiff没有assembly prepare/commit/abort或generation release事务。

Runtime 使用单调时钟计算内部 absolute deadline。该 absolute deadline 不暴露给用户代码；用户可见的是 `TimeoutError` 的 budget / source 语义。

Deadline 到达且 block / request 尚未结束时，对应语义结果立即固定为 `TimeoutError` 或平台 timeout error，未完成 work item 收到runtime内部停止信号。外层代码不等待尽力清理完成，清理或底层operation晚到的值、错误和 Skiff 可见写入被丢弃。“立即”表示语义结果立即确定，不表示 OS socket、数据库请求或纯 CPU 指令在同一个机器指令内物理停止。

Runtime / compiler 必须让纯 Skiff CPU 代码可被有界停止。停止检查至少出现在函数入口、loop 条件求值前、loop backedge、每个 lane 开始前和完成后、`concurrent value` tail lane 开始前，以及可能长时间运行的生成代码片段中。

内部停止是request/lane生命周期控制，不生成用户可捕获的错误，也不存在`CancelError` public type、
用户源码cancel API、按request id取消API或runtime stop inspection API。Deadline与内部停止不同：
有效deadline到达产生可捕获的`TimeoutError`；内部停止本身没有业务payload。

Internal transport的`request.cancel`、`connection.request.cancel`等既有命名可以暂时保留为
best-effort stop hint。发送方不得依赖peer接收hint才释放本地pending，接收方必须幂等处理；
hint丢失只可能多消耗资源，不能改变业务正确性或恢复晚到结果。

外部operation一旦开始，内部停止不承诺物理中断或回滚。尚未poll的operation可以直接丢弃；
已经poll且可能越过外部副作用点的operation可以在自身既有deadline内尽力完成，也可能留下unknown
outcome，runtime不得把它伪装成“已撤销”。晚到结果不能重新写入已结束的request heap/env。
Transaction、lease、stream和file staging等资源由各自owner定义正常terminal；异常停止只要求
best-effort清理且不能无限保留request-local状态。若底层无法及时清理，使用driver/session关闭、
lease TTL或其它已有平台回收机制，不为可见request等待cleanup acknowledgement。

## 9. Stream semantics

Skiff 当前只支持 server / source stream。Stream 是一次性值，不是持久化数据结构。

`Stream<T>` 可以出现在 service operation，或adapter kind明确允许stream的external ingress entry最外层
返回类型，表示server stream；当前HTTP ingress中只有`rawHttp`允许精确返回
`Stream<std.http.HttpResponseStreamEvent>`，`typedJson`始终是unary。`Stream<T>`也可以作为显式
stream-producing native std / package API 的返回类型，表示 request-local external source handle。平台 std 可以返回包含 stream 字段的 runtime-owned handle record，例如 `std.http.HttpClientStreamHandle.body`；这个 record 只能在当前 request 中使用，不能持久化或作为业务协议 schema。

`Stream<T>` 当前不能作为用户 operation 参数、用户 record 字段、持久化字段、collection 元素或普通 public API type 字段。平台 std 的 runtime-owned handle 字段和 native host operation 参数是特权例外；例如 `std.file.createFromStream(source: Stream<bytes>, ...)` 在同一 request 内消费 source，不把 stream 传出为远程 API 或 durable value。普通 Skiff package / local function 不能通过源码 body 创建独立、可逃逸的 stream source。

返回`Stream<T>`的service operation或允许stream的external gateway entry是server-stream producer。
Producer共享当前request frame、deadline、trace、call stack和request heap。函数体内`emit expr`要求
`expr`可赋给`T`，并向当前stream sink写一个ordered chunk。函数体自然结束或裸`return`表示stream
normal end；`return expr`在server-stream handler中是编译错误。当前不提供`Stream<T, R>`或stream完成后的
独立response值。

`emit` 是 backpressure point。Consumer 不读取、gateway / client 断开或 buffer 达到平台上限时，当前 request 必须暂停、内部停止或按平台错误结束，不能无限积压。`emit` 不允许出现在 `concurrent` surface 内；`concurrent value` 的 tail lane 也属于该 surface。需要并发计算后输出时，先在 lane 中计算值，等 `concurrent` block 结束后，在后续顺序代码中按确定顺序 emit。

调用方把 `Stream<T>` 当作只能顺序消费的一次性值。每次迭代读取下一个 item；chunk 产生一次 loop body 执行；end 使 loop normal exit；error 映射为当前 lane 中的 ordinary throw，可被外层 `catch<E>` 捕获。已经处理过的 chunk 不回滚。

`break`、`return`、timeout或ancestor内部停止必须结束当前stream consumption，并向source发送
best-effort stop hint。Stream被消费、结束或停止后不能再次迭代，也不能复制到多个lane同时消费。

跨 service / gateway stream 使用 runtime transport 的 stream ordering。Request-local external stream 使用对应 primitive 的顺序读取语义。

`std.http.stream` 返回 response handle，其 `body` 是 request-local external source stream；`std.http.sse`、LLM stream 等 native std / package API 也返回 request-local external source handle。调用方提前退出、当前request timeout、ancestor内部停止或consumer `break`时，runtime尽力abort in-flight external request；底层不支持时直接丢弃后续response，并由HTTP client owner按既有operation deadline收束。

`std.file.createFromStream` 是 native host operation consumer：它只接受 `Stream<bytes>`，在当前 request 中顺序读取 chunk、写入不可变文件 staging，并在 source end 后提交文件。source error、request内部停止或host operation error必须停止相对侧并清理未提交staging；已经提交的不可变文件不伪装成可回滚。

External stream source error 映射为当前 lane 的 ordinary throw。若 server-stream operation 正在消费 external stream 并转换输出，inner stream error 会让当前 server-stream request error，除非用户源码捕获并收敛。

## 10. HTTP entry

HTTP entry 是gateway-selected external HTTP dispatch，不是service-to-service API。External caller既可以是
浏览器、移动端或CLI客户端，也可以是支付回调等第三方服务器；反向代理和上游HTTP/SSE转发同样属于该入口。
Skiff service之间的调用始终使用ServiceContract，不通过HTTP entry伪装。

Router根据trusted service/version header经release pointer选择精确deployment build，再按该deployment内的
method/path和loaded gateway entry metadata调用HTTP handler。该
handler由`http.yml`选择，不要求进入`api.yml`或ServiceContract。Router不按display/source path猜target，
也不根据HTTP Host、content-type或业务payload自行解释或选择target。

外部 HTTP request 在 dispatch 前打包为标准 HTTP request envelope；method、url、path、query、headers 和 body 保持为业务可检查的数据。Query 和 headers 使用数组保留重复项和顺序。

Raw HTTP handler返回单个`std.http.HttpResponse`时，gateway写回status、headers和body；返回
`Stream<std.http.HttpResponseStreamEvent>`时，runtime把`start/chunk/end` event转换成
`response.start/response.chunk/response.end` frame，gateway按顺序写socket。后一种是external HTTP
server stream，适合在不聚合完整body的情况下转发上游HTTP/SSE；external caller只观察普通HTTP协议，
不需要理解Skiff的`Stream<T>`。`start`前的runtime error写platform JSON error；`start`后的error首轮按
连接中断处理。Client disconnect表示response consumer已经消失；Router可以向runtime发送内部
best-effort stop hint，但不产生公开cancel response，也不承诺撤销handler已经提交的副作用。

Typed HTTP route 是 compiler-generated unary wrapper，不是 router framework。Router 先按严格
service/version header选择exact deployment，再在该deployment内按method/path选择route并发起HTTP
dispatch；wrapper 在 service runtime 内执行 `http.pre`、JSON body decode、handler 调用和 HTTP 200 JSON encode。`typedJson` handler不能返回任意`Stream<T>`；需要保留原始request bytes/headers做签名校验、控制status/response headers、转发binary body或按到达顺序转发body chunk时必须声明`rawHttp`。越过 wrapper 的 `std.http.HttpError`、decode error 或平台错误通过 runtime `response.error` 映射为非 2xx platform error response。该 HTTP response body 固定为 JSON `{ "message": string, "detail": Json? }`，不暴露 internal `code` 或业务指定 status；平台策略选择 status，例如 body/schema decode 为 400、handler / `http.pre` 未捕获异常为 500、timeout 为 504、runtime/dependency unavailable 为 503。

HTTP status code本身不是throw；业务代码必须检查status。HTTP entry的可观测target id是gateway entry
target。Router作为external connection owner按operator-owned平台HTTP request上限生成request deadline；
service配置不能收紧、放宽或伪造该上限。Method/path/handler mapping变化是
deployment/ingress配置变化，不改变service protocol identity；HTTP Host映射属于Router外部ingress，
不进入service route或deployment identity。

## 11. WebSocket entry

WebSocket entry主要属于客户端直连的API层service。下游业务service不拥有Connection，也不把WebSocket
当作service-to-service transport。

WebSocket物理连接由gateway/hub维护。Upgrade先按严格service/version header经release pointer选择精确
deployment build，并在该deployment内按path选择entry。Connection拥有connection id、service id、状态、
business identity、entry identity、精确deployment build和物理socket；它不需要一个service-call protocol
operation identity。

Connect operation是一次request frame，用于连接验证和connection policy初始化。连接建立后，service可
发送单向notification，或通过`std.websocket.requestJsonToConnection`向精确connection发起request并等待
peer response。Peer也可以调用`websocket.yml.jsonRpc`显式声明的method；用户不声明一个接收所有raw
frame的`receive`函数。未声明request method不进入业务代码；第一版所有notification都不进入业务代码。

Router中的专用WebSocket request broker拥有编码无关的request identity、pending生命周期和
connection/socket-generation归属；第一版`jsonrpc-2.0-text` adapter解释JSON-RPC 2.0控制字段。业务`method`、
`params`、`result`与error `data`保持opaque。Response只有在
`(outbound, connectionId, socket/generation identity, profile, id)`精确命中pending request时才转回原
runtime execution；它不创建新的runtime ingress request。Request按
`(inbound, websocketEntryId, pinned deployment build, socket generation, profile, method)`解析gateway entry，经
`RuntimeDispatcher`创建独立request frame。Raw text/binary send不选择RPC配置；未来binary RPC必须是
另一个显式配置。

Inbound handler的`params`按linked adapter plan decode，return按linked type编码为JSON-RPC `result`；
handler只能unary return。Platform parse/invalid/method/params/internal错误使用`-32700`、`-32600`、
`-32601`、`-32602`、`-32603`，容量和timeout使用`-32000`、`-32001`。业务未捕获throw
一律脱敏为Internal error；预期失败使用typed result union。

Service端主动推送必须显式调用`std.websocket.send*`；需要peer结果时显式调用
`requestJsonToConnection`。后者是潜在suspension point，并继承当前execution deadline与内部停止状态。
本地deadline或内部停止会删除pending并丢弃晚到response；第一版不向peer发送JSON-RPC取消notification。
deadline仍产生`TimeoutError`。平台transport correlation不取代业务幂等、durable run或tool attempt
identity，也不提供自动retry。

WebSocket连接按已冻结的connection protocol identity、deployment build与socket generation路由；
request/response不能跨connection、build或socket generation恢复。第一版不接受peer request cancellation；
peer disconnect表示该socket上没有response consumer，runtime内部停止仍在执行的inbound handler并失败
outbound pending。Release pointer更新不迁移既有socket；它继续绑定旧build，直到drain或断开。Connection
drain的等待预算属于Router lifecycle owner，不能被external request `requestTimeoutMs`覆盖。

## 12. Effect metadata at runtime

Effect metadata 是 compiler / publisher / runtime 对调用影响的共享语义数据。Host operation 是 runtime 执行边界：一次调用离开纯 Skiff 求值，访问宿主能力、外部系统或跨 service transport。

Runtime 使用 effect metadata 解释调用属于 local read/write、external read/write 还是 telemetry / host write；解释 stable target id、timeout aggregation target、conflict-key、idempotency、并发策略、测试替身和观测事件匹配。

Metadata 是语义承诺，不是日志注释。Metadata 缺失、target / conflict-key无法静态确定或provenance不明时，compiler / runtime必须保守拒绝不安全并发或重试。第一版不要求每个operation公开声明通用
cancel-safety、commit point或cleanup action；资源owner在具体实现中负责自己的正常terminal与异常
best-effort收束。

在 `concurrent` sibling lane 中，read-only external effect 可以和其他 read-only external effect 并发；external write 默认不能和 sibling external effect 并发；同一 conflict-key 上的 read/write 或 write/write 不能位于不同 sibling lane；标记 `exclusive` 的 operation 在当前 request 内不能和任何 sibling external effect 并发。

只有metadata显式声明concurrency safe并提供可比较conflict-key时，runtime才能允许更宽松并发。
需要顺序执行多个冲突effect时，源码应把它们放到同一个`serial { ... }` lane或普通顺序代码中。

`config.require` / `config.optional`读取当前精确deployment中为Package slot烘焙的局部`ConfigView`，是本地
只读访问，不是外部I/O。源码path相对当前Package分区，不能越过Package ID边界。Array、
Map、scalar receiver方法只产生local read/write effect；mutation按writable place或`inout` path参与
并发检查，pure static transform只读取输入snapshot。
`std.json.encode` / `std.json.decode`是boundary codec helper，不访问外部系统。

`std.http.request`、`std.http.stream`、`std.http.sse`、service call、telemetry emit、WebSocket send 和
WebSocket request是host operation或跨runtime operation，必须有effect metadata。
`std.websocket.sendJson<T>`若只编码JSON并调用sendText，则host write发生在sendText；helper本身可以
作为wrapper暴露高层target / timeoutTarget，但不能隐藏底层trace和external effect事实。普通WebSocket send是
non-suspending；`std.websocket.requestJsonToConnection`必须标记`maySuspend`并传播deadline与内部停止状态。

## 13. Current unsupported runtime capabilities

当前不支持：

- `detach`、后台 lane 或 request 结束后继续运行的 Skiff coroutine。
- 顶层 mutable 容器、语言级共享内存或跨 request heap handle。
- 本地 Skiff stream producer / coroutine；普通 Skiff 函数不能返回自己创建的可逃逸 `Stream<T>`。
- 双向 stream、用户 operation stream 参数、半关闭、reconnect / resume、持久化 stream cursor 或 `Stream<T, R>`。
- `Stream<T>` 作为用户 operation 参数、用户 record 字段、持久化字段、collection 元素或普通 public API type 字段。
- 语言级 snapshot / read view 表达式。
- request-scope component，或任何把request-local状态保存进deployment-scoped/process-global mutable
  singleton的机制。
- queue、cron、async task 和 durable long-running workflow。
- 用户可调用的request cancellation、按request id取消或runtime stop inspection API。
- WebSocket JSON-RPC peer `$/cancelRequest`与`-32800 Request cancelled`。
- 函数体级 `concurrent` modifier。
- `concurrent` surface 内的普通控制语句、直接 throw/catch、`timeout(...)`、`with`、stream control、`dispatch`、嵌套 `concurrent` 和 callback / anonymous function body。
- `return` 穿过 `concurrent` 边界；`break` / `continue` 穿过不在同一 lane 内的 loop 边界。
- Set surface、string indexing 和 heap cycle。
- 自动 retry 非幂等 operation。
- Router core 中的业务 host/path route bind、cookie/session/auth 解释或业务WebSocket消息协议解释。
  专用、版本化的WebSocket RPC编码adapter可以解释自己的framing和控制字段，但不能解释业务payload。

这些限制是当前 runtime 合约的一部分，不应由 std wrapper、package wrapper 或 router 配置绕过。

未来支持这些能力时，必须显式定义其 request 生命周期、heap/resource provenance、effect metadata、
timeout、内部停止和边界 schema 规则，不能隐式复用当前 request frame 语义。
