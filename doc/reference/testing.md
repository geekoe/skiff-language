# Skiff Testing Reference

本文负责测试源码、测试发现、runtime 执行语义、package 测试、测试service配置profile和
production artifact 边界。本文不负责具体 CLI flag、测试进程编排、secret文件分发或runner的实现细节。

## 1. Testing Surface

Skiff 只保留一种测试用例语义：`test` block。unit、integration 和 live smoke 不是不同
语法或执行宿主，只是测试范围、effect policy 和 runtime target ownership 的差异。

规则：

- `test` block 只允许出现在 `*.test.skiff` 文件中。
- `assert` 只允许出现在 `test` block 中。
- 生产文件是所有不以 `.test.skiff` 结尾的 `.skiff` 文件。
- 生产文件中的普通 declaration 都进入生产编译产物，即使它只被测试使用。
- test-only declaration 只参与测试编译，不进入 production artifact、package assembly、
  service assembly、public API surface 或 config metadata。

## 2. Test-Only Source

`*.test.skiff` 是测试专用 source file。它可以包含测试用例、helper、fixture type、测试
专用 import，以及可选的 `test defaultRun false` directive。

`test defaultRun false` 是文件级测试发现 directive：

- 默认值是 `true`。
- 只影响目录输入的默认发现。
- 显式指定该 test-only 文件时，runner 必须运行它。
- 它不改变 runtime target ownership、network permission、config 注入或 live key policy。
- 它只接受 literal bool，不接受表达式、config 或运行时条件。

## 3. Test Service And Visibility

测试源码属于独立 test service，不再作为被测 package 的 source overlay 编译。test service
使用普通 PackageArtifact、ServiceContract、Deployment 和 RuntimeAssembly 格式；不定义
TestServiceArtifact。

`service.yml` 必须声明：

```yaml
id: example.com/widget-tests
kind: test
```

`kind: test` 是 authoring/workflow 约束：

- 只允许 `skiff test` 构建和运行；
- 普通 package publish、service publish/deploy 和 production watch 拒绝 test service；
- artifact、linker、loader 和 Runtime 使用普通格式与执行路径；
- test service 的 config profile固定为`skiff-test`，来自`config.skiff-test.yml`；
- 本机或部署时的私密覆盖使用同profile的`config.skiff-test.secret.yml`，该文件不得提交；
- 同一个test service在一次runner execution中只编译一次`PackageArtifact`，所有selected cases共享
  authored config layers和dependency graph。非live runner按发现顺序把case装入有界batch；每个batch的
  generated deployments作为多个root进入自己的`RuntimeAssembly`，并把各自隔离的snapshot分区写入一个
  batch-local `RuntimeConfigSnapshot`。一个case只属于一个batch，并观察该batch generation钉住的两个ref；
- 每个case仍有独立synthetic `ServiceDeployment`、`ServiceContract`、gateway entry/ingress binding、
  系统派生service数据库、heap、effect registry和execution nonce。共享assembly/snapshot record不等于
  共享deployment、ConfigView或mutable state；
- 需要不同配置时使用另一个test service，不提供per-case config override，也不允许调用方切换
  test service config profile。

测试service配置profile与runtime目标environment是两个概念：

- `skiff-test`固定选择测试service的配置和secret overlay；
- live runner的target environment标识外部Router/Runtime中的activation generation，可能是`dev`或
  其它部署环境；
- target environment不得反向选择`config.<environment>.yml`。普通隔离测试中两者通常都叫
  `skiff-test`，不能因此在实现里合并两个owner。

test service dependency 可以声明：

```yaml
packages:
  - id: example.com/widget
    version: 1.0.0
    alias: widget
    topLevelAlias: widgetImpl
```

同一dependency entry提供两条互不回退的名字：

- `alias`始终解析该dependency的`api.yml` public paths，测试service与普通consumer规则相同；
- 可选`topLevelAlias`只解析同一精确implementation artifact的source module/top-level symbol index；
- `topLevelAlias`只允许出现在`kind: test` service。它必须是合法、唯一的identifier，并且不能与当前
  manifest中任何package/service `alias`或其它`topLevelAlias`冲突；
- public alias与top-level alias没有public-first、top-level-first、fallback或precedence；
- 可访问的顶层名字包括同一文件中的type、function、const、interface和附着到type的`db object`；
  DB attachment不需要、也不会因为测试访问而进入`api.yml`；
- 通过`topLevelAlias`取得的精确implementation type值可以调用该type在同一精确artifact中已有的
  impl methods。method仍属于receiver的method namespace，不能写成新的顶层路径；generic receiver
  保留完整type arguments。

这项test-only能力不会扩大普通`alias`或service boundary。普通`alias`返回的public/local-closure-only
type值仍只能使用API公开的public instance methods；不能借返回值发现任意package-local impl method。
service call返回的对象也不会获得provider package-local method namespace。

旧`access: topLevel`字段和“以普通alias替换public解析面”的语义均已删除；出现旧字段必须在manifest
读取时失败，不能兼容、忽略或改写。

`topLevelAlias`路径语法为：

```text
<top-level-alias>/<source-module-path>.<top-level-name>
```

`root.*` 始终表示 test service 自己。测试访问被测 package 必须写成例如
`widgetImpl/internal.codec.decodeForTest(...)`；访问其公开API仍写
`widget/<api-public-path>`。两条路径都属于同一dependency edge、`PackageRequirement`和
`PackageBinding`，不会生成第二个requirement、code slot或collection projection。

DB target使用同一语法，例如`db require widgetImpl/model.User(id)`。它只允许解析到该精确dependency
artifact中`model.User`的type及其同文件`db object User` attachment；所有read/write、query、
`db claim`和`db lease` target都遵守这条规则。`db transaction`本身没有target，transaction内的每个
DB operation分别解析。缺少type、attachment或精确artifact约束时编译/链接失败，不能按短名或其它
dependency中同module/type的声明回退。

top-level权限不传递。被测package的public ABI可以正常闭合其dependency公开类型，但这不授予test
service对transitive dependency内部顶层符号的访问权。例如：

```text
aihub-tests -> aihub -> llm-providers
```

`aihub-tests`可以通过`aihub`的普通alias使用AIHub公开面及其ABI中引用的公开类型；若测试源码还要直接
访问`llm-providers`的顶层符号，必须在自己的manifest中再声明一条指向`llm-providers`的direct
dependency，并在该entry设置`topLevelAlias`。

这会形成direct与transitive两条真实dependency edge。对于声明DB metadata的Package，activation把精确
provider metadata链接到当前test service由系统派生的唯一数据库。这里“一条active projection”只表示
一个最终生效的Package/schema/collection metadata owner，不表示创建额外数据库：

- 两条edge解析到同一精确`PackageBuild`且owner facts相同时，合并为一个active projection；
- 同一Package ID解析到不同build，拒绝；
- logical collection identity缺失/重复或system physical-name encoding collision，拒绝。

`config.skiff-test.yml`根部按canonical Package ID分区；direct/transitive edge相同不会创建第二份
ConfigView、数据库或metadata owner。

## 4. Test Discovery

测试必须显式启动。runner 输入可以是普通 source file、test-only source file 或目录。

普通 source file 输入：

- 运行该 source file 所属 service / package 中默认发现的测试。
- 与目录输入一样跳过 `defaultRun false` 文件；不按 source file 名称匹配 test-only 文件。

test-only source file 输入：

- 只运行该文件中的测试。
- 显式指定文件时不受 `defaultRun false` 跳过。
- 测试编译仍包含所属 service / package 的全部 production 顶级符号。

目录输入：

- 递归发现 `*.test.skiff`。
- 跳过 `defaultRun false` 文件。
- 跳过 generated / dependency 目录，例如 `target`、`node_modules` 和 dot directory。

## 5. Runtime Execution And Effect Policy

所有 Skiff 测试源码都由 `skiff-test-runner` 编译，并在真实 Skiff runtime 进程中执行。Skiff
不提供 compiler VM / unit 执行模式；unit、integration 和 live smoke 只描述测试范围，不改变
执行语义。测试级别由 effect policy 和 runtime target ownership 决定，不由语法、目录名或
文件名决定。

非 live 测试：

- 普通 `skiff test <path>` 为整个命令创建一套隔离 router / runtime，并在其中运行全部 case。
- 仓库 canonical Skiff 源码套件为整个 registry plan 创建一套隔离 router / runtime，并在所有
  registry entry 之间复用该进程。
- runner对同一个普通`kind: test` service只执行一次package compile、config layer读取和dependency
  graph resolve。非live case按`relative_path`文件顺序贪心装箱，每个activation batch硬上限16个case：
  一个不超过上限的文件保持完整，只有单文件超过16个case时才按上限切分。每个batch的独立synthetic
  deployments作为roots链接成一个multi-root `RuntimeAssembly`，并把对应隔离配置分区写入一个
  `RuntimeConfigSnapshot`，再由一次activation transaction并列提交两个ref。单个case不另有assembly或
  generation；live显式文件执行仍使用一个activation，不采用non-live batch上限。
- authored config layers与runner保留overlay在分批前读取并冻结一次；每个batch从同一份内存snapshot投影
  ConfigView，执行期间不得重读磁盘config或观察不同authored snapshot。
- runner在开始任何activation前完成全部batch的assembly/config projection和artifact publication。每个
  batch使用唯一execution scope，并从调用者提供的同一base assembly/config pair独立投影；后一个batch
  不得把前一个test batch当成base或累积其generated deployments。activation、readiness和case dispatch
  随后按batch顺序串行执行，generation从上一个成功commit的精确值安全递增。
- runner从本次隔离activation的受信target environment写入snapshot顶层；Runtime必须在物化任何case
  `ConfigView`前验证它与activation environment精确相等。
- 不访问真实网络或外部服务；外部 effect 必须由 test double 替换，缺失 double 必须失败。
- runner负责构造逐case synthetic deployment、contract、gateway entry/ingress binding和root request
  frame；package测试由runner自动生成临时test service及其共享multi-root assembly activation。
- runner在对应generated deployment的snapshot分区中增加
  `skiff.test.ingressUrl`动态只读overlay；Package不读取ambient environment，authored文件不能覆盖它。
- runtime进程和batch内assembly activation复用不扩大可变状态生命周期。每个case的数据库identity由
  `(testRunId, generatedTestServiceId)`系统派生，其ConfigView、
  heap、effect registry、execution nonce和synthetic deployment资源仍按runner isolation contract
  独立finalize；batch artifacts和activation只由该test service execution统一清理。

每次root test dispatch都新建一个不透明`testCaseCapability`。它只标识该case execution的effect与
生命周期authority，不替代deployment selector，也不从assembly generation派生。该值只存在于
Router/Runtime Host的测试传输与注册状态，不是Skiff值、config或用户可见effect API。普通production
request及其Actor调用不携带该capability。

root发起的direct spawn、任意深度recursive spawn、同步Actor method call与
`spawn actor.method(...)`都是同一case的派生请求。测试运行时为它们携带父请求的同一
capability和当前active parent request id；Router只从同一Runtime session上仍active的父请求
授权派生，capability token本身不足以授权。派生请求不得新建capability、借用其它root的
capability或在父请求终结后迟到加入。另一个case的root dispatch即使属于同一assembly，也必须
获得不同值。

携带test capability的Actor method首版必须与active parent属于同一service，并在父请求的精确
origin Runtime connection上执行，以共享该Runtime内存中的case effect registry。目标Actor属于
其它service、已归其它Runtime、origin connection已断开或owner在admission期间改变时都必须
fail closed；测试语义不承诺跨service或Runtime共享test effects。已admit的Actor method一直属于
该case，直到其terminal execution结束；root finalization必须等待它结束，但拒绝父请求结束后才
到达的新Actor child。这项test-only约束不改变production Actor语义：不携带test capability的
跨service Actor spawn仍按普通Actor owner routing合法执行。

activation generation推进不会重绑已active的test Actor execution。它的同步Actor call与Actor spawn继续
使用进入该execution时已固定的capability、parent chain、assembly/deployment与generation authority。
该authority仍是current generation时，Actor发起的self-ingress完整继承它；Actor已属于old generation时，
self-ingress必须在路由前fail closed。runner不为此保留或构造历史gateway route snapshot，也不得把
old-generation execution重绑到current route。

### 5.1 HTTP entry tests

测试真实HTTP entry时，test service显式声明自己的`http.yml`，entry引用`*.test.skiff` wrapper。
wrapper可以通过普通alias或`topLevelAlias`调用被测代码；runner不会自动投影被测对象的
production `http.yml`。

测试源码使用现有HTTP client：

```skiff
const baseUrl = config.require<string>("skiff.test.ingressUrl")
const response = std.http.request(std.http.HttpClientRequest {
  method: "POST",
  url: baseUrl.concat("/chat/events"),
  headers: headers,
  body: body,
  timeoutMs: null,
})
```

`skiff.test.ingressUrl`是non-live runner通过普通resolved config view注入的保留只读path；authored
配置不得声明或覆盖它。它是普通绝对`http` URL，不是特殊scheme，也不能从环境变量或固定端口猜测。
`std.http.request`返回普通`HttpClientResponse`；`std.http.stream`返回普通
`HttpClientStreamHandle`。没有测试专用HTTP API或类型。

当且仅当case运行于runner拥有的隔离test execution，且request URL的canonical origin精确等于该
ingress URL时，该调用是self-ingress：

- 在inline effect匹配前识别，因此父调用本身不消费`std.http.request`/`std.http.stream` double；
- 测试执行适配自动使用当前case唯一service id和contract version，测试代码不能提供或覆盖selector；
- Router按普通HTTP ingress规则路由，Host不参与选择；
- Router消费并剥离`x-skiff-test-case-capability`与
  `x-skiff-test-case-parent-request-id`，再在nested `request.start`中传递严格校验的
  capability/parent pair；Host只把capability-only frame视为root，携带pair的frame必须从仍active的
  parent建立derived execution，不得重复创建root case；
- entry内部的outbound effects继续使用父case同一个inline-effect registry；
- current-generation test Actor的self-ingress及其direct/recursive spawn、同步Actor method call与
  Actor method spawn完整继承父root的derived authority，不得因共享assembly或activation generation
  附着到另一个case；old-generation Actor可继续其immutable direct/spawn chain，但self-ingress fail closed；
- 同一case第一版禁止两个active self-ingress请求。stream EOF、失败或consumer drop/break才释放
  active状态。

其它origin仍是普通outbound HTTP，必须遵守non-live double/network policy。用户headers若包含
`x-skiff-service`、`x-skiff-version`、`Host`、`Content-Length`、`Transfer-Encoding`或任一
hop-by-hop header，发送前失败；header name按大小写不敏感比较。

流式测试按完整body或应用协议frame断言，不按传输chunk断言。SSE必须先把连续bytes解析成完整event；
consumer `break`或drop沿普通HTTP client disconnect链取消Router/runtime中的子请求。子请求不独立
执行setup/finalization；父case始终是唯一effect检查和资源finalization owner。

Live smoke：

- 同样在真实 runtime 进程中执行，但 target 由调用者显式提供和拥有，并允许显式授权的外部
  effect。
- live只改变runtime target ownership和effect policy；test service仍固定读取
  `config.skiff-test.yml`及可选`config.skiff-test.secret.yml`。
- activation URL、ingress URL、artifact root、target environment与expected generation是每次运行的
  显式target参数，不属于test service config，也不能写进secret overlay。
- 应使用 `defaultRun false` 并通过文件路径运行。
- 没有 live key 时应 skip，而不是失败。
- 只验证真实外部服务的少量关键路径，不替代 unit / integration 覆盖。

## 6. Package Tests

package 测试由归属该 package 仓库的 test service 承载。test service 把被测 package 声明为
精确dependency；需要内部顶层访问时在同一entry设置`topLevelAlias`，普通`alias`仍可访问公开API。

规则：

- test helper 只进入 test service artifact，不进入被测 package production artifact；
- 测试通过普通config snapshot、dependency、contract、deployment和assembly机制运行；
- package 内部测试使用top-level alias调用被测实现，不使用overlay `root.*`；
- Package 仍不是远程 service；本机 Package call 不得伪装成 service-to-service RPC；
- public API、implementation top-level、manifest 或 shared helper 变化时运行对应 test
  services。

## 7. Inline Test Effects

effect doubles 写在所属 test block 中，不使用外部 `skiff.test-doubles.json`。

规范形态：

```skiff
test "request succeeds" effects {
  std.http.request {
    expect: {
      method: "POST",
      url: "https://example.test",
    },
    respond: {
      status: 200,
      headers: Array.empty<std.http.HttpHeader>(),
      body: bytes.fromUtf8("ok"),
    },
  }
} {
  // assertions
}
```

单次调用使用 `respond`、`throw` 或 `stream`。多次调用统一使用 `sequence`，每一步可以
声明自己的 request subset 和一种结果：

```skiff
test "unary and stream sequences" effects {
  dependency.request {
    expect: {
      method: "POST",
    },
    sequence: [
      {
        expect: { url: "https://example.test/first" },
        respond: { status: 503 },
      },
      {
        expect: { url: "https://example.test/second" },
        throw: RequestFailure { code: "denied" },
      },
      {
        expect: { url: "https://example.test/third" },
        respond: { status: 200 },
      },
    ],
  },
  dependency.events {
    sequence: [
      {
        expect: { url: "https://example.test/events/first" },
        stream: [{ value: "first" }, { value: "second" }],
      },
      {
        expect: { url: "https://example.test/events/second" },
        throw: RequestFailure { code: "disconnected" },
      },
    ],
  },
  dependency.failure {
    throw: RequestFailure { code: "denied" },
  },
} {
  // assertions
}
```

`respond`、`throw`、`stream` 与 `sequence` 互斥；一个 target 必须且只能声明其中一个
结果字段。`sequence` 必须非空，每一步可以声明一个可选 `expect`，并且必须且只能声明
`respond`、`throw` 或 `stream` 之一，但只能使用该 target 签名允许的结果。普通 unary
target 的步骤只能是 `respond` 或签名声明的 typed `throw`；直接返回 `Stream<T>` 的
target 只能是 `stream` 或签名声明的 typed `throw`。不把 unary response 隐式解释成
stream，也不把 `respond` 隐式解释成 `Stream<T>` 的单个 event 或完整 stream。target
顶层 `expect` 是每一步都必须满足的公共 request subset；step `expect` 是该次调用额外
必须满足的 subset。两者分别匹配并取逻辑 AND，不做 object merge 或覆盖。序列和 stream
event 表使用 effect DSL 的 `[item, ...]`，不是 Skiff 通用 array literal。顶层
`expect` 表达式在 setup 中只求值一次；Runtime 保存其 wire 快照，并对 sequence 的每一步
复用同一快照。

规则：

- compiler 必须解析精确 effect target，并静态检查 expect/respond/typed error/stream event；
- compiler 可以把 `effects` block 降低为 test-only hidden setup callable，但 setup 不是独立
  request，也不创建另一份 heap、activation 或 execution nonce；
- runner 对一个 case 只创建一次执行上下文，并在其中依次执行 setup 和 test body；setup
  产生的 response、error 和 stream event 必须立即按 linked target type plan
  materialize 到该 case 的 effect registry，不能把 heap value 作为跨执行共享对象保存；
- root dispatch为该执行上下文新建`testCaseCapability`；direct/recursive spawn、同步Actor method call和
  Actor method spawn必须由同Runtime session上的active parent派生，并继承父请求的同一
  capability，因此共享该case的registry与finalization owner，但不与同一assembly内其它root共享；
- setup 成功后才执行 test body；setup 失败时 body 不执行；
- case finalization 是 runtime-owned teardown phase。无论 body 成功、assert 失败、throw、
  timeout 或 cancel，都必须检查未消费 double、销毁 registry 并释放 case 资源；
- self-ingress HTTP子请求复用同一个registry且不触发finalization；只有父case执行上述phase；
- 当前没有用户可写的 teardown 语法。未来若增加 teardown，它是同一 case execution 中位于
  body 之后、runtime finalization 之前的独立 phase，不改变现有 `effects` surface；
- effect declaration 只属于当前 case，case 完成后 registry 销毁；
- expected request subset 在真实 typed value materialization 后匹配；
- 同一个精确 linked target 在一个 case 中只能声明一次；不同 alias 如果解析到同一个
  Package callable 或 service operation，也必须拒绝并要求写成一个显式 `sequence`；
- sequence 不能为空，未消费或超量调用必须产生明确测试失败；
- double 执行仍参与 effect conflict 和 capability policy；
- 不提供 JSON manifest compatibility loader；旧文件和旧 schema 必须迁移或删除。

## 8. AI / CI Selection

AI 和 CI 不需要测试配置文件来决定默认测试。它们按改动范围显式选择文件或目录。

原则：

- 改生产文件，运行所属 service / package 目录，或显式选择受影响的 test-only 文件。
- 改 test-only 文件，运行该 test-only 文件。
- 改 package public API、manifest 或 shared helper，运行受影响 package 的测试。
- 改 runtime effect、config、HTTP 编码、router activation，运行相关 integration 测试。
- live smoke 只在用户显式要求、nightly 或 release 验证流程中运行。

Runner flag只控制runtime target ownership、target environment/generation和effect policy，不改变
测试源码语义，不选择test service config profile，也不把`defaultRun false`文件加入目录默认发现。
非live与live都不切换到compiler VM。

## 9. Production Artifact Boundary

production build 必须满足：

- 生产文件中出现 `test` block 是编译错误。
- `*.test.skiff` 不进入 production source set。
- test-only code 不进入 file artifact bytecode、service assembly 或 package assembly。
- test-only config reads 不进入 production config use metadata。
- test-only declarations 不进入 production package API 或 service protocol surface。
- test-only helper 不影响 package / service identity。
- test-only `root.*` reference 不参与 production root reference validation。
- `test defaultRun` directive 不进入 production artifact。

test service 使用普通 artifact 格式，但 production publish/deploy workflow 必须根据
`kind: test` 拒绝它。Runtime 格式无需测试特例；测试权限在 compiler 名称解析阶段已经关闭。
