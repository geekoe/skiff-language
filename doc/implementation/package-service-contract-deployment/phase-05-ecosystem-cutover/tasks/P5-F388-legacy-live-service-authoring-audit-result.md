# P5-F388 legacy live service authoring audit result

## 结论

本审计只读覆盖以下三个legacy live service root，以及它们的两个直接package dependency：

- `runtime/encrypted-storage-live/default-service`
- `runtime/encrypted-storage-live/mapped-service`
- `runtime/live-tests`
- `runtime/encrypted-storage-live/package-store/example~com~~encrypted-live-store/1.0.0`
- `runtime/live-tests/package-store/example~com~~runtime-live-kit/1.0.0`

审计基线为commit `b910dd734f099f96423b297fc5049e30a776428f`、tree
`bf3d163e71db434f91b324df61e7056b7935a0e3`。本轮没有修改上述production source、没有启动
stable instance/MongoDB/router/runtime、没有请求activation/ingress、没有连接任何外部服务。

三个service root可以收敛为canonical `package.yml + api.yml + service.yml + config.dev.yml`。
最终外部面精确为：

| service | contract operations | HTTP gateway entries | raw unary | raw server stream |
| --- | ---: | ---: | ---: | ---: |
| `example.com/encrypted-live-default@0.1.0` | 0 | 21 | 21 | 0 |
| `example.com/encrypted-live-mapped@0.1.0` | 0 | 13 | 13 | 0 |
| `skiff.run/runtime-live@0.1.0` | 0 | 6 | 5 | 1 |
| **总计** | **0** | **40** | **39** | **1** |

但是当前HEAD不能把这份设计直接称为可live cutover，结论为**NO-GO，需先完成三个fail-closed前置项**：

1. accepted `package.yml`字段`collection_name_mapping`在compiler pipeline中被丢弃，mapped
   package的`package_secret -> mapped_package_secret`语义不会进入artifact、deployment或runtime
   activation metadata；
2. 当前test runner的package-test gateway仍显式fail-closed，且runtime-live plan没有传
   `--base-assembly`；现有`--config`、`--allow-network`与per-test `requestPayload` /
   `expectedError`机制均已退出canonical CLI；
3. `runtime/live-tests/package-store`位于service root内部，普通package source递归会把其中
   `http.skiff`误编进`skiff.run/runtime-live`实现；runtime test-only DB schema又与production
   `package.yml` state requirement无法同时通过校验。

这些都不是需要产品取舍的开放问题。collection mapping的既有可观察语义、canonical
base-assembly继承以及package source ownership已经明确；应fail closed修复后再取得fresh
authoring receipt。

## 1. canonical ownership与source facts

### 1.1 encrypted default

`runtime/encrypted-storage-live/default-service/internal/live.skiff`是实现owner。它声明四个本地DB
object：

- `Credential`（`apiKey`与`refreshToken` encrypted）；
- `CredentialArchive`（`apiKey` encrypted）；
- `RotationBarrier`；
- `IdentityDateRecord`。

它没有package dependency。旧`service.yml`中的`id`、`version`分别迁移到canonical
`service.yml`和`package.yml`；`timeout.default: 120000`迁移到`config.dev.yml`。目标控制文件为：

```yaml
# runtime/encrypted-storage-live/default-service/package.yml
id: example.com/encrypted-live-default
version: 0.1.0
state:
  encrypted-live-default-store:
    kind: database
```

```yaml
# runtime/encrypted-storage-live/default-service/api.yml
{}
```

`config.dev.yml`必须绑定`encrypted-live-default-store`到该service原有数据库namespace，并拥有
`timeout: 120000`、service principal、quota和lifecycle policy。若保留
`internal/encrypted.live.test.skiff`的secret marker，production normal source必须用private
accessor声明`encryptedLive.testRunnerSecret` requirement，profile才可合法绑定该固定测试值；
不得再通过runner `--config`注入。

### 1.2 encrypted mapped与encrypted store package

`runtime/encrypted-storage-live/mapped-service/internal/live.skiff`拥有本地`RotationBarrier`与
`Credential` DB object；后者显式collection name为`Credential`。其五个package-store
handler经alias `encryptedStore`调用`example.com/encrypted-live-store@1.0.0`。

依赖package的`store.skiff`拥有`PackageSecret`，显式collection name为`package_secret`；
`api.yml`公开`insertOne`、`readOne`、`scan`、`rewrite`、`rewriteBatch`五个callable。目标manifest
为：

```yaml
# runtime/encrypted-storage-live/package-store/
#   example~com~~encrypted-live-store/1.0.0/package.yml
id: example.com/encrypted-live-store
version: 1.0.0
state:
  encrypted-live-store:
    kind: database
```

```yaml
# runtime/encrypted-storage-live/mapped-service/package.yml
id: example.com/encrypted-live-mapped
version: 0.1.0
state:
  encrypted-live-mapped-store:
    kind: database
packages:
  - id: example.com/encrypted-live-store
    version: 1.0.0
    alias: encryptedStore
    collection_name_mapping:
      package_secret: mapped_package_secret
```

```yaml
# runtime/encrypted-storage-live/mapped-service/api.yml
{}
```

当前legacy `collectionNameMapping`不是当前input model接受的字段；当前精确accepted spelling为
`collection_name_mapping`。mapped deployment必须把`encrypted-live-mapped-store`与dependency
的`encrypted-live-store`两个database state key都绑定到原mapped service database namespace，
从而保留“service-local collection与package collection位于同一service DB，后者改名为
`mapped_package_secret`”的既有语义。timeout同样只由`config.dev.yml`拥有。

这里存在一个阻断性实现缺口：

- `compiler/input-model/src/dependencies.rs`会解析`collection_name_mapping`；
- `compiler/driver/pipeline/mod.rs::package_requirement`构造`PackageRequirement`时不携带它；
- `artifact-model/src/compile_requirements.rs::PackageRequirement`没有mapping字段；
- generated `PackageBinding`也没有该事实；
- `runtime/host/src/loader/active_assembly_context.rs::activation_db_metadata`只复制dependency
  artifact中的原始`collection_name`。

因此当前构建即使接受上述YAML，也会静默使用`package_secret`。前置修复必须把mapping作为
exact caller dependency edge的已验证事实贯穿requirement/artifact binding/link/activation，
参与相关artifact identity，并在缺失、重复、unknown source collection或target collision时
fail closed。

### 1.3 runtime live service与runtime kit

`runtime/live-tests/internal/http_adapter.skiff`拥有五个现有implementation handler：

- `rawEcho(HttpRequest) -> HttpResponse`
- `typedJsonEcho(HttpRequest) -> HttpResponse`
- `binaryEcho(HttpRequest) -> HttpResponse`
- `guardedPost(HttpRequest) -> HttpResponse`
- `streamEcho(HttpRequest) -> Stream<HttpResponseStreamEvent>`

名称`typedJsonEcho`不改变adapter contract：它在function内部手工执行
`std.http.decodeJson`，目标gateway仍是`kind: rawHttp`，不是`typedJson`。

目标package ownership为：

```yaml
# runtime/live-tests/package.yml
id: skiff.run/runtime-live
version: 0.1.0
state:
  runtime-live-store:
    kind: database
packages:
  - id: example.com/runtime-live-kit
    version: 1.0.0
    alias: runtimeKit
```

```yaml
# runtime/live-tests/api.yml
{}
```

runtime kit必须从会被service source递归的`package-store`移到source collector明确跳过的
`.skiff-packages`，并改成独立canonical package：

```yaml
# runtime/live-tests/.skiff-packages/
#   example~com~~runtime-live-kit/1.0.0/package.yml
id: example.com/runtime-live-kit
version: 1.0.0
```

```yaml
# runtime/live-tests/.skiff-packages/
#   example~com~~runtime-live-kit/1.0.0/api.yml
packageEcho: http.packageEcho
```

现有kit `http.skiff`保持其public implementation。旧`package.yml`内嵌
`api: { "": http }`必须删除；public API只由`api.yml`拥有。

`RuntimeLiveDoc`及其DB object当前只存在于
`internal/db_live.live.test.skiff`。普通package编译排除`.test.skiff`，所以：

- 不在`package.yml`声明database state时，test overlay因使用DB schema而失败；
- 声明`runtime-live-store`时，production package又因没有normal-source DB schema而失败。

最小修复是把`RuntimeLiveDoc`、DB object及DB probe helpers移到normal private module
`internal/db_live.skiff`，测试文件只保留assertions，并把DB target改为该module的exact root
selector。类似地，file helper与测试所需config accessors需要进入normal private source，使
base deployment真实拥有DB/file capability及全部config requirements；不能靠test overlay
凭空扩张production owner。

## 2. exact HTTP gateway matrix

以下约束适用于表中每一行，且是逐entry验收的一部分：

- `host`均为`"*"`；
- `kind`均为`rawHttp`；
- 唯一adapter arg均为
  `param: request, source: { kind: http.request }`；
- 除`runtime.stream`外，精确signature均为
  `(request: std.http.HttpRequest) -> std.http.HttpResponse`，dispatch为`unary`；
- `runtime.stream`精确signature为
  `(request: std.http.HttpRequest) -> Stream<std.http.HttpResponseStreamEvent>`，dispatch为
  `serverStream`；
- selector、key、handler、guard、pre和implementation build不进入gateway entry identity，
  但仍必须逐字段出现在contract/deployment receipt验收中。

| # | service / suggested key | host | method | path | guard | exact private handler | params → return | dispatch |
| ---: | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | default / `default.insert` | `*` | POST | `/encrypted-live/default/insert` | `internal.live.guard` | `internal.live.insertOne` | `HttpRequest → HttpResponse` | unary |
| 2 | default / `default.insert-many` | `*` | POST | `/encrypted-live/default/insert-many` | `internal.live.guard` | `internal.live.insertMany` | `HttpRequest → HttpResponse` | unary |
| 3 | default / `default.insert-bulk` | `*` | POST | `/encrypted-live/default/insert-bulk` | `internal.live.guard` | `internal.live.insertBulk` | `HttpRequest → HttpResponse` | unary |
| 4 | default / `default.read` | `*` | POST | `/encrypted-live/default/read` | `internal.live.guard` | `internal.live.readOne` | `HttpRequest → HttpResponse` | unary |
| 5 | default / `default.project` | `*` | POST | `/encrypted-live/default/project` | `internal.live.guard` | `internal.live.projectOne` | `HttpRequest → HttpResponse` | unary |
| 6 | default / `default.replace-key` | `*` | POST | `/encrypted-live/default/replace-key` | `internal.live.guard` | `internal.live.replaceByKey` | `HttpRequest → HttpResponse` | unary |
| 7 | default / `default.replace-query` | `*` | POST | `/encrypted-live/default/replace-query` | `internal.live.guard` | `internal.live.replaceByQuery` | `HttpRequest → HttpResponse` | unary |
| 8 | default / `default.upsert` | `*` | POST | `/encrypted-live/default/upsert` | `internal.live.guard` | `internal.live.upsertOne` | `HttpRequest → HttpResponse` | unary |
| 9 | default / `default.update` | `*` | POST | `/encrypted-live/default/update` | `internal.live.guard` | `internal.live.updateOne` | `HttpRequest → HttpResponse` | unary |
| 10 | default / `default.scan` | `*` | POST | `/encrypted-live/default/scan` | `internal.live.guard` | `internal.live.scan` | `HttpRequest → HttpResponse` | unary |
| 11 | default / `default.rewrite` | `*` | POST | `/encrypted-live/default/rewrite` | `internal.live.guard` | `internal.live.rewrite` | `HttpRequest → HttpResponse` | unary |
| 12 | default / `default.rewrite-batch` | `*` | POST | `/encrypted-live/default/rewrite-batch` | `internal.live.guard` | `internal.live.rewriteBatch` | `HttpRequest → HttpResponse` | unary |
| 13 | default / `default.identity-date` | `*` | POST | `/encrypted-live/default/identity-date` | `internal.live.guard` | `internal.live.identityDate` | `HttpRequest → HttpResponse` | unary |
| 14 | default / `default.archive-insert` | `*` | POST | `/encrypted-live/default/archive-insert` | `internal.live.guard` | `internal.live.insertArchive` | `HttpRequest → HttpResponse` | unary |
| 15 | default / `default.archive-read` | `*` | POST | `/encrypted-live/default/archive-read` | `internal.live.guard` | `internal.live.readArchive` | `HttpRequest → HttpResponse` | unary |
| 16 | default / `default.archive-scan` | `*` | POST | `/encrypted-live/default/archive-scan` | `internal.live.guard` | `internal.live.scanArchive` | `HttpRequest → HttpResponse` | unary |
| 17 | default / `default.archive-rewrite` | `*` | POST | `/encrypted-live/default/archive-rewrite` | `internal.live.guard` | `internal.live.rewriteArchive` | `HttpRequest → HttpResponse` | unary |
| 18 | default / `default.archive-rewrite-batch` | `*` | POST | `/encrypted-live/default/archive-rewrite-batch` | `internal.live.guard` | `internal.live.rewriteArchiveBatch` | `HttpRequest → HttpResponse` | unary |
| 19 | default / `default.archive-restore` | `*` | POST | `/encrypted-live/default/archive-restore` | `internal.live.guard` | `internal.live.restoreArchive` | `HttpRequest → HttpResponse` | unary |
| 20 | default / `default.barrier` | `*` | POST | `/encrypted-live/default/barrier` | `internal.live.guard` | `internal.live.activateBarrier` | `HttpRequest → HttpResponse` | unary |
| 21 | default / `default.barrier-status` | `*` | POST | `/encrypted-live/default/barrier-status` | `internal.live.guard` | `internal.live.barrierStatus` | `HttpRequest → HttpResponse` | unary |
| 22 | mapped / `mapped.insert` | `*` | POST | `/encrypted-live/mapped/insert` | `internal.live.guard` | `internal.live.insertOne` | `HttpRequest → HttpResponse` | unary |
| 23 | mapped / `mapped.read` | `*` | POST | `/encrypted-live/mapped/read` | `internal.live.guard` | `internal.live.readOne` | `HttpRequest → HttpResponse` | unary |
| 24 | mapped / `mapped.scan` | `*` | POST | `/encrypted-live/mapped/scan` | `internal.live.guard` | `internal.live.scan` | `HttpRequest → HttpResponse` | unary |
| 25 | mapped / `mapped.rewrite` | `*` | POST | `/encrypted-live/mapped/rewrite` | `internal.live.guard` | `internal.live.rewrite` | `HttpRequest → HttpResponse` | unary |
| 26 | mapped / `mapped.rewrite-batch` | `*` | POST | `/encrypted-live/mapped/rewrite-batch` | `internal.live.guard` | `internal.live.rewriteBatch` | `HttpRequest → HttpResponse` | unary |
| 27 | mapped / `mapped.service-probe-insert` | `*` | POST | `/encrypted-live/mapped/service-probe-insert` | `internal.live.guard` | `internal.live.insertServiceContextProbe` | `HttpRequest → HttpResponse` | unary |
| 28 | mapped / `mapped.service-probe-read` | `*` | POST | `/encrypted-live/mapped/service-probe-read` | `internal.live.guard` | `internal.live.readServiceContextProbe` | `HttpRequest → HttpResponse` | unary |
| 29 | mapped / `mapped.service-probe-scan` | `*` | POST | `/encrypted-live/mapped/service-probe-scan` | `internal.live.guard` | `internal.live.scanServiceContextProbe` | `HttpRequest → HttpResponse` | unary |
| 30 | mapped / `mapped.service-probe-rewrite` | `*` | POST | `/encrypted-live/mapped/service-probe-rewrite` | `internal.live.guard` | `internal.live.rewriteServiceContextProbe` | `HttpRequest → HttpResponse` | unary |
| 31 | mapped / `mapped.service-probe-rewrite-batch` | `*` | POST | `/encrypted-live/mapped/service-probe-rewrite-batch` | `internal.live.guard` | `internal.live.rewriteServiceContextProbeBatch` | `HttpRequest → HttpResponse` | unary |
| 32 | mapped / `mapped.service-probe-restore` | `*` | POST | `/encrypted-live/mapped/service-probe-restore` | `internal.live.guard` | `internal.live.restoreServiceContextProbe` | `HttpRequest → HttpResponse` | unary |
| 33 | mapped / `mapped.barrier` | `*` | POST | `/encrypted-live/mapped/barrier` | `internal.live.guard` | `internal.live.activateBarrier` | `HttpRequest → HttpResponse` | unary |
| 34 | mapped / `mapped.barrier-status` | `*` | POST | `/encrypted-live/mapped/barrier-status` | `internal.live.guard` | `internal.live.barrierStatus` | `HttpRequest → HttpResponse` | unary |
| 35 | runtime / `runtime.echo` | `*` | POST | `/runtime-live/echo` | — | `internal.http_adapter.rawEcho` | `HttpRequest → HttpResponse` | unary |
| 36 | runtime / `runtime.json` | `*` | POST | `/runtime-live/json` | — | `internal.http_adapter.typedJsonEcho` | `HttpRequest → HttpResponse` | unary |
| 37 | runtime / `runtime.binary` | `*` | POST | `/runtime-live/binary` | — | `internal.http_adapter.binaryEcho` | `HttpRequest → HttpResponse` | unary |
| 38 | runtime / `runtime.guarded` | `*` | GET | `/runtime-live/guarded` | — | `internal.http_adapter.guardedPost` | `HttpRequest → HttpResponse` | unary |
| 39 | runtime / `runtime.stream` | `*` | POST | `/runtime-live/stream` | — | `internal.http_adapter.streamEcho` | `HttpRequest → Stream<HttpResponseStreamEvent>` | serverStream |
| 40 | runtime / `runtime.package` | `*` | POST | `/runtime-live/package` | — | `internal.http_adapter.packageEcho` | `HttpRequest → HttpResponse` | unary |

`runtime.guarded`的gateway selector故意是GET；`guardedPost`内部调用
`std.http.requireMethod(request, "POST")`并返回405。这是handler行为验证，不是gateway
`guard`，不可在迁移中把它“修正”为POST或添加gateway guard。

### 2.1 identity grouping

39条unary entry必须共享：

```text
skiff-gateway-entry-v1:sha256:02fc7a22e177f3c1cf768d65f53c2a15e874d8a8aeee67510a72746c0514c940
```

其exact identity preimage为：

```json
{"schema":"skiff-gateway-entry-identity-v1","surface":{"externalErrorProjection":{"kind":"fixed","version":"v1"},"protocol":{"kind":"http","surface":{"adapterKind":"rawHttp","dispatchMode":"unary","externalSources":[{"kind":"http.request"}],"requestBodySchema":null,"responseSchema":null,"streamItemSchema":null}}}}
```

唯一`runtime.stream`必须使用：

```text
skiff-gateway-entry-v1:sha256:ee0272ceb804a70357e8b4b48d6ff4b1161eb6709b1cf5d537396a5cf5aacd62
```

其exact identity preimage为：

```json
{"schema":"skiff-gateway-entry-identity-v1","surface":{"externalErrorProjection":{"kind":"fixed","version":"v1"},"protocol":{"kind":"http","surface":{"adapterKind":"rawHttp","dispatchMode":"serverStream","externalSources":[{"kind":"http.request"}],"requestBodySchema":null,"responseSchema":null,"streamItemSchema":{"branches":[{"fields":{"headers":{"items":{"fields":{"name":{"kind":"string"},"value":{"kind":"string"}},"kind":"record","required":["name","value"]},"kind":"array"},"status":{"kind":"integer"},"tag":{"kind":"stringLiteral","value":"start"}},"kind":"record","required":["headers","status","tag"]},{"fields":{"tag":{"kind":"stringLiteral","value":"chunk"},"value":{"kind":"bytes"}},"kind":"record","required":["tag","value"]},{"fields":{"tag":{"kind":"stringLiteral","value":"end"}},"kind":"record","required":["tag"]}],"kind":"closedUnion"}}}}}
```

## 3. service.yml shape、wrapper与guards

每个旧`http.routes`数组必须改为stable named map。代表性entry为：

```yaml
id: example.com/encrypted-live-default
http:
  default.insert:
    host: "*"
    method: POST
    path: /encrypted-live/default/insert
    kind: rawHttp
    handler: internal.live.insertOne
    guard: internal.live.guard
    adapterArgs:
      - param: request
        source:
          kind: http.request
```

三个`service.yml`均不得再包含`version`、`packages`、`routes`、global `http.guard`或deployment
timeout。所有handler/guard selector都必须是current-package selector，不得带legacy `root.`，
也不得直接引用dependency alias。

### 3.1 runtimeKit.packageEcho private wrapper

当前旧route直接使用`runtimeKit.packageEcho`，canonical gateway authoring会拒绝dependency-qualified
handler。exact source owner必须是当前实现package的
`runtime/live-tests/internal/http_adapter.skiff`，增加：

```skiff
function packageEcho(request: std.http.HttpRequest) -> std.http.HttpResponse {
  return runtimeKit.packageEcho(request)
}
```

`service.yml`只选择`internal.http_adapter.packageEcho`。wrapper是private implementation detail，
不得出现在`runtime/live-tests/api.yml`；kit的`api.yml`仍公开
`packageEcho: http.packageEcho`供wrapper使用。这既保留package call coverage，也保证gateway
resolver只绑定exact current implementation package。

### 3.2 encrypted guards

default与mapped的`internal.live.guard`已经具有canonical精确signature：

```skiff
function guard(request: std.http.HttpRequest) -> std.http.HttpResponse? {
  return null
}
```

authoring model原生支持per-entry `guard`，所以34条encrypted entry逐条填写
`guard: internal.live.guard`即可；无需`pre`、无需新helper、无需扩大API。旧top-level guard不可被
保留为global字段。

两个source中的`writerBarrier`是mutation handler内部的选择性业务屏障。它只覆盖应被rotation
barrier阻止的write路径，不能提升为覆盖所有entry的gateway guard；否则read/status等既有行为
会改变。

## 4. zero-operation contract与fresh authoring receipt

三个service implementation package的`api.yml`均为`{}`，因此每个generated
`ServiceContract`必须满足：

- exact coordinate分别为上述service id `@0.1.0`；
- `operations == {}`；
- `packageTypeRequirements == []`；
- 无operation schema、无公开callable；
- 40条HTTP entry属于deployment gateway/ingress surface，不是contract operation。

每个generated `ServiceDeployment`必须满足：

| service | operation bindings | gateway entries | ingress bindings | root implementation |
| --- | ---: | ---: | ---: | --- |
| encrypted default | 0 | 21 | 21 | exact default package build |
| encrypted mapped | 0 | 13 | 13 | exact mapped package build |
| runtime live | 0 | 6 | 6 | exact runtime-live package build |

fresh build-only应从空的临时artifact root一次性处理五个package root，逻辑依赖顺序为：

1. `example.com/encrypted-live-store@1.0.0`；
2. `example.com/runtime-live-kit@1.0.0`；
3. `example.com/encrypted-live-default@0.1.0`；
4. `example.com/encrypted-live-mapped@0.1.0`；
5. `skiff.run/runtime-live@0.1.0`。

必须使用current `skiff dev sync --build-only --environment dev --artifact-root <fresh>`路径；所有
root作为同一次sync的显式roots，不能再调用legacy `skiff-dev-sync.mjs --build-root`、
`--default-packages-dir`或`--no-reload`。build-only不得调用
`requestAssemblyActivation`，也不得触达activation/ingress。

receipt验收不能只看数组长度。必须逐个打开receipt给出的immutable `recordPath`并验证：

- 5个package publication record及exact id/version/build/ABI/requirements；
- 3个zero-operation service contract record；
- 3个deployment record及`0/21/13/6` operation/gateway计数；
- 40个ingress selector、private handler、guard、adapter arg、dispatch与上述两组identity；
- 1个assembly record，exact root deployments为3个、reachable package closure为5个；
- mapped dependency edge携带并实际应用collection mapping；
- build-only期间environment generation和active assembly保持不变。

若mapping事实没有进入record并改变identity，receipt必须判失败，不能以“HTTP route count正确”
替代存储语义证据。

## 5. test/harness与production live边界

### 5.1 encrypted storage harness

`scripts/lib/encrypted-storage-live-harness.mjs`当前`seedServiceArtifacts`仍调用已退出的sync参数，
`encryptedStorageTestRunnerArgs`仍生成`--allow-network`与`--config`。canonical target必须：

1. 在fresh artifact root用上述build-only路径发布encrypted store、default、mapped（若独立跑
   encrypted lane则是这3个root；共享S6 gate则与runtime两root一起为5个）；
2. 从receipt读取exact assembly identity；
3. runner参数只使用
   `--artifact-root`、`--base-assembly`、`--platform-source-root`、`--live`、
   `--activation-url`、`--ingress-url`、`--environment`、`--expected-generation`、
   `--deny-skips`、`--require-tests`；
4. base assembly的business deployments必须在test overlay activation后仍存在；
5. 启动临时Mongo/runtime、activation、HTTP请求、raw Mongo验证、key rotation与crash/recovery
   全部保持manual/live tier，只有用户明确授权后才能执行。

default test的secret marker应成为base deployment的normal config requirement与固定dev
binding；`serviceDb.mongoUrl`属于runtime/state binding，不是runner `--config`。

### 5.2 runtime-live plan与tests

`scripts/lib/verify-live-plan.mjs`当前只要求activation URL、ingress URL、artifact root、
environment与expected generation，生成的test-runner命令没有`--base-assembly`。
`CanonicalBaseAssembly::load(..., None)`只得到空base，因而无法继承root deployment的config、
state、runtime capability与service ownership。plan必须从fresh authoring receipt接收exact
runtime-live assembly identity、先验证该record，再把它逐条传为
`--base-assembly <identity>`。

当前`verify-live-plan`把“root缺package.yml”当作预期失败的fixture测试，也必须翻转为：

- 三个root均有canonical control files的positive discovery；
- receipt中的root/contract/deployment/assembly exact owner positive assertion；
- 不接受legacy `service.yml` shape、missing package owner或empty base assembly。

runtime test source还需要以下canonicalization：

- `RuntimeLiveDoc` schema/probes移到normal source，`package.yml`声明`runtime-live-store`；
- operation/db/file marker及file capability由normal private source声明，base deployment拥有对应
  config/capability；test-only source只做assert；
- 动态router/ingress URL不写入静态`config.dev.yml`。production route round trip由使用runner
  `--ingress-url`的live executor/checker负责；
- `operation.live.test.skiff`的`__skiffPayload`目前只会收到runner硬编码的empty payload。
  要么S7提供canonical per-case payload input并进入receipt，要么明确把case降为empty-payload
  boundary test；旧JSON `requestPayload`不会被读取；
- 64 MiB file negative case产生平台级`ResourceLimitExceeded`。该错误的catch projection为
  `None`，不能伪装成`catch<std.file.FileError>`；S7需提供显式expected platform error
  receipt/checker。旧JSON `expectedError`不会被读取；
- package-test ingress目前在
  `test-runner/src/package_test_assembly.rs`因non-empty overlay bindings显式fail-closed；在S7
  完成synthetic gateway projection前，不得报告runtime live test可执行。

`runtime/live-tests/runtime-live.config.example.json`因此应删除；它既不是canonical
`config.<environment>.yml`，也不是current runner input。

### 5.3 non-live acceptance

S6 authoring节点可自行执行且只允许执行：

- manifest/source parser与compiler unit tests；
- gateway projection/identity tests；
- harness/verify-live-plan的pure argument与receipt tests；
- fresh temporary artifact root上的`dev sync --build-only`；
- immutable record内容与环境未激活断言。

以下证据明确不属于non-live acceptance：

- 启动或重启stable instance、MongoDB、router、runtime、telemetry；
- 调用`/__skiff/activate-assembly`或任何ingress；
- runtime-live外部URL；
- encrypted keyring、raw Mongo、rotation、crash/recovery；
- 用本机stable artifacts或已有environment generation替代fresh receipt。

## 6. exact implementation write set

本节是后续实现owner的最小write set，本审计未修改其中任何文件。

### 6.1 S6 canonical authoring owner

新增：

- `runtime/encrypted-storage-live/default-service/package.yml`
- `runtime/encrypted-storage-live/default-service/api.yml`
- `runtime/encrypted-storage-live/default-service/config.dev.yml`
- `runtime/encrypted-storage-live/mapped-service/package.yml`
- `runtime/encrypted-storage-live/mapped-service/api.yml`
- `runtime/encrypted-storage-live/mapped-service/config.dev.yml`
- `runtime/live-tests/package.yml`
- `runtime/live-tests/api.yml`
- `runtime/live-tests/config.dev.yml`
- `runtime/live-tests/internal/db_live.skiff`
- `runtime/live-tests/internal/file_live.skiff`（若采用move而非同等normal private fixture module）
- `runtime/live-tests/.skiff-packages/example~com~~runtime-live-kit/1.0.0/api.yml`

修改：

- `runtime/encrypted-storage-live/default-service/service.yml`
- `runtime/encrypted-storage-live/default-service/internal/live.skiff`
  （只增加test config private accessor时）
- `runtime/encrypted-storage-live/mapped-service/service.yml`
- `runtime/encrypted-storage-live/package-store/example~com~~encrypted-live-store/1.0.0/package.yml`
- `runtime/live-tests/service.yml`
- `runtime/live-tests/internal/http_adapter.skiff`
- `runtime/live-tests/internal/operation.skiff`
- `runtime/live-tests/.skiff-packages/example~com~~runtime-live-kit/1.0.0/package.yml`
- `runtime/live-tests/.skiff-packages/example~com~~runtime-live-kit/1.0.0/http.skiff`
  （move，content无需行为改动）
- `scripts/lib/encrypted-storage-live-harness.mjs`
- `scripts/lib/verify-live-plan.mjs`
- `scripts/tests/encrypted-storage-live-harness.test.mjs`
- `scripts/tests/verify-live-plan-platform-source.test.mjs`
- 与runtime-live plan exact receipt/base-assembly相关的现有script test文件

删除（由move或retired input产生）：

- `runtime/live-tests/package-store/example~com~~runtime-live-kit/1.0.0/package.yml`
- `runtime/live-tests/package-store/example~com~~runtime-live-kit/1.0.0/http.skiff`
- `runtime/live-tests/runtime-live.config.example.json`

### 6.2 shared mapping checkpoint owner

只拥有compiler/artifact/deployment/runtime mapping transport与对应tests，不触碰上述三个service
root。至少覆盖：

- `compiler/input-model/src/dependencies.rs`
- `artifact-model/src/compile_requirements.rs`
- compiler pipeline/generated deployment的requirement与binding projection
- linker/loader的dependency-edge validation
- `runtime/host/src/loader/active_assembly_context.rs`
- collection mapping identity、collision与activation metadata tests

具体类型可由owner放在最小正确边界；硬约束是mapping不能只停留在authoring input或diagnostic
metadata。

### 6.3 S7 package-test/live evidence owner

只拥有test overlay/gateway/runtime execution与`.live.test.skiff`语义，不改S6 control files：

- `test-runner/src/package_test_assembly.rs`及其tests；
- runner canonical per-case input/expected platform error（若保留两项覆盖）；
- `runtime/live-tests/internal/db_live.live.test.skiff`
- `runtime/live-tests/internal/file_live.live.test.skiff`
- `runtime/live-tests/internal/http_adapter.live.test.skiff`
- `runtime/live-tests/internal/operation.live.test.skiff`
- `runtime/encrypted-storage-live/default-service/internal/encrypted.live.test.skiff`

如S7需要改plan/harness参数接口，应由S6先固定接口，S7只消费，避免两个节点同时编辑
`verify-live-plan.mjs`或`encrypted-storage-live-harness.mjs`。

## 7. minimal no-overlap DAG

不建议把所有缺口塞进一个实现节点。最小无重叠DAG为：

```text
C0 collection-mapping transport/admission
                    │
                    ▼
S6 one authoring owner for all 3 service roots + 2 package roots
                    │
                    ├── fresh build-only receipt gate
                    ▼
S7 package-test gateway/base-assembly/per-case evidence
                    │
                    ▼
explicitly authorized live execution (no source ownership)
```

关键边界：

- **S6内部不再拆三名root owner**。default、mapped、runtime及两套script plan由一个owner修改，
  避免40-entry inventory、shared receipt与harness参数发生交叉漂移；
- C0只解决共享collection mapping语义，可先于S6，也可与S6代码准备并行，但S6 receipt gate依赖
  C0；
- S7只解决canonical test overlay/live evidence，依赖S6的exact base assembly；
- 最后live execution只消费已提交、fresh、exact receipt，且必须再次取得用户授权；它不拥有
  production source。

因此实施决策是：**三root authoring采用单一S6 owner；跨层compiler mapping与S7 live runner
采用两个独立前后置节点。** 这是满足既有语义、避免文件重叠且不把live授权混入authoring
acceptance的最小切分。
