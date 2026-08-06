# P5-F384 Test assembly gateway control-plane audit result

状态：Completed（只读审计；冻结 HTTP test/control 后继边界）。

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 1. 审计基线与结论

| 项目 | 值 |
| --- | --- |
| audited commit | `2c87c3585bfbdcfdd41f415864cc8c91aa0e9a91` |
| audited tree | `f4b1cb7c0748128e994fcba6c1a1c3a8ccaaef9e` |
| worktree | `/Users/geek/workspace/skiff-p5-f384-test-gateway-control-audit` |

本审计没有修改 Skiff production/test source，没有运行 stable/live，也没有派子 Agent。任务列出的
`test-runner/src/bin/ecosystem_smoke_fixture.rs`并不存在；实际 fixture owner 是
`test-runner/src/ecosystem_smoke_fixture.rs`，它的 binary consumer 是
`test-runner/src/bin/package_service_smoke_fixture.rs`。这是路径勘误，不是设计缺口。

结论如下：

1. 必须先完成一个 Router shared checkpoint，再迁移 test-runner consumer。不能只改
   `package_test_assembly.rs`：当前 Router test control path既读已经退出模型的operation字段，又没有安全的
   `testEffectsEnabled = true` dispatch authority。
2. package-test每个case需要一个HTTP gateway entry；ecosystem HTTP fixture总计需要两个：
   package-test entry一个、`POST /probe` entry一个。可共享同一个typed-null unary builder，但两个handler
   return schema不同，因此两个`GatewayEntryIdentity`不同。
3. package-test与ecosystem synthetic `ServiceContract`均应为零operation；gateway handler必须是exact
   private `InternalFunction`，不得把现有public test/marker重新伪装成service operation。
4. test doubles已经由test overlay编译进inline setup。control JSON和F359 binary wire都必须彻底删除
   `testEffectDoubles`；隔离的`kind: "test"` control path只把`testEffectsEnabled: true`写入F359 header。
5. production `assemblyHttpRequestHeader`继续固定`testEffectsEnabled: false`；普通
   `RuntimeDispatcher.dispatchBinary`和`AssemblyRuntimeRegistry.validateDispatchRequest`继续拒绝true。
6. 不需要用户决策。F359、F365与当前gateway artifact模型已经足以冻结实现；WebSocket业务路由继续由独立
   后继处理，不能阻塞这里的两个HTTP entry。

## 2. 当前精确调用链与断点

### 2.1 Package-test

当前调用顺序为：

```text
discover_package_test_cases
  -> compile_package_test_overlay
     -> package_test_ast_for_cases
        -> private <test>Setup(): void
           (CompilerTestEffectRegister statements)
        -> private <test>(): void
           (first statement calls <test>Setup)
     -> PackageTestOverlayBinding { case, public_path, callable_id }
  -> assemble_package_test_fixture_for_run
     -> FAIL at package_test_assembly.rs:77-80
```

因此F381动态测试当前尚未到artifact publish、activation或Router。若临时越过第一个fail-closed，休眠中的旧
producer仍按以下顺序工作：

```text
compile_package_test_contract
  -> synthesize stable operation "run"
  -> compile_contract
package_test_operation_inputs
  -> currently FAIL at :239-252
  -> historically produced one ServiceDeploymentOperationInput
     and POST /__skiff/package-test/{index} old operation ingress
package_test_deployment_input
  -> operationBindings = old operation
  -> gatewayEntries = {}
CanonicalPackageTestEntrypoint
  -> { case, selector, deployment, contract, operation }
```

当前精确旧命中为：

| 文件/行 | 旧事实 | canonical替代 |
| --- | --- | --- |
| `package_test_assembly.rs:37-44` | entrypoint保存`ServiceContractRef`和`ContractOperationId` | 保存`deployment + gatewayEntryKey + gatewayEntryIdentity + selector + mode` |
| `:77-80` | nonempty overlay统一fail closed | 逐case构造reference-closed zero-op contract/deployment/gateway |
| `:97-106` | 编译contract并取得operation inputs | 编译zero-op contract并取得private wrapper gateway facts |
| `:128-140` | 从`deployment.operation_bindings[0].contract_operation_id`回填receipt | 从`deployment.gateway_entries[key]`回填exact key/identity/mode |
| `:179-236` | `compile_package_test_contract`制造stable key `run` operation和schema closure | zero-op contract、empty package type requirements/records |
| `:239-252` | 旧operation/ingress producer现在显式报错 | one gateway entry + one selector/key ingress |
| `:365-413` | deployment input接收`operation_bindings`且写`gateway_entries: {}` | `operation_bindings: []`且写one-entry map |

`test_overlay.rs:326-507`已经证明doubles的owner在compiled program：每个effect变成
`Stmt::CompilerTestEffectRegister`，test body先调用hidden setup。后继只需给现有零参test body增加一个
private HTTP wrapper；不得重新序列化effect内容。

### 2.2 Ecosystem HTTP fixture

`ecosystem_smoke_fixture.rs:57-64`目前无条件fail closed。其休眠实现仍执行：

```text
assemble_package_test_fixture
  -> one package-test old operation entrypoint
compile_smoke_contract
  -> public marker becomes contract operation
  -> optional public websocket becomes second contract operation
smoke_deployment_input
  -> operationBindings(marker[, websocket])
  -> gatewayEntries = {}
  -> ingress = {}
entrypoint receipt
  -> unary/websocket each carry contract + operation
```

精确旧命中为：

| 文件/行 | 旧事实 | HTTP后继 |
| --- | --- | --- |
| `ecosystem_smoke_fixture.rs:11-18` | 导入`ContractOperationId`、`ServiceDeploymentOperationInput`、`compile_contract` | HTTP projection不再导入operation authoring |
| `:39-45` | `EcosystemSmokeEntrypoint`保存contract/operation | HTTP entry保存deployment/key/identity/selector/mode |
| `:67-155` | dead implementation把package test、unary和optional WS都映射为operation entrypoint | 只恢复package-test与unary两个HTTP projection；WS保持独立未决 |
| `:157-217` | marker/WS进入operation bindings；gateway map为空 | smoke deployment为zero-op + one `/probe` gateway |
| `:219-280` | `SmokeContract`保存marker/WS operation id | zero-op contract，不再从operations回读id |
| `package_service_smoke_fixture.rs:229-260` | binary为三个entrypoint输出`contract`/`operation` | HTTP receipt输出key/identity/mode；不得把未路由的WS伪装成canonical entrypoint |

HTTP-only完成态中，ecosystem assembly有两个deployment root、两个zero-op contract和两个HTTP gateway
ingress。optional `/socket`不计入这两个gateway entry，也不得暂时保留旧operation ingress。现有完整
WebSocket smoke在WS owner完成前仍应明确fail closed；focused HTTP fixture不得因此继续fail closed。

### 2.3 Runtime execution与Router

package-test artifact成功组装后的当前链为：

```text
CanonicalTestRecords.publish
  -> POST /__skiff/activate-assembly
  -> strict activation receipt
  -> readiness poll
  -> execute_entrypoints
  -> POST /__skiff/test-dispatch
  -> AssemblyControlPlane.handleTestDispatch
  -> active snapshot ingress lookup
  -> assemblyHttpRequestHeader
  -> RuntimeDispatcher.dispatchBinary
  -> AssemblyRuntimeRegistry.pickDispatchConnection
  -> F359 request.start binary frame
  -> F365 Host exact route admission
  -> runtime HTTP gateway eval
```

现状在control request两端仍是旧合同：

- `test-runner/src/runtime_execution.rs:175-183`发送
  `kind: "runtimeAssembly"`、`contractOperationId`、selector、空payload、`testEffectsEnabled: true`和
  `testEffectDoubles: {}`；`:196-205`只看control endpoint外层HTTP status，没有验证runtime的
  `response.end.httpResponse.status`。
- `router/src/router/assemblyControlPlane.ts:101-110`按selector lookup后读取已经不存在的
  `binding.contractOperationId`。
- 同文件`:113-128`向F364 production header builder传不存在的
  `testEffectsEnabled`、`testEffectDoubles`、`callerTarget`参数；`:173-253`的parser仍声明并接受这些
  旧字段，还会lowercase host、uppercase method，因而不是exact-match parser。
- `assemblyHttpGateway.ts:221-269`已经只生成F359 header，但固定
  `testEffectsEnabled: false`。这个production默认是正确的。
- `assemblyRuntimeRegistry.ts:512-579`已经逐值验证F359
  assembly/generation/selector/gateway identity/mode/httpRequest，但`:521-525`正确地让普通dispatch拒绝
  任意true。因此control path不能只把header flag翻转后调用普通dispatcher。
- F365 Host只接受F359 header，并把flag传给request/eval；Host
  `assembly_request_adapter`以empty doubles构造request，`runtime/request/src/http_gateway_execution.rs:87-94`
  仅在flag为true时创建test-enabled interpreter。这条线不需要恢复wire doubles。

## 3. 邻接旧命中分类

以下命中必须区分，不能为了让一次反搜为零而误改WebSocket/general legacy：

| 命中 | 分类/处置 |
| --- | --- |
| `router/src/router/controlPlane.ts`中的`testEffectDoubles` | general legacy `RouterControlPlane`；production `server.ts`实际安装`AssemblyControlPlane`。不属于本HTTP assembly checkpoint |
| `router/tests/test-dispatch.test.ts`旧double正例 | 上述general legacy owner的直接测试，不作为assembly test path正例 |
| `assemblyRuntimeRegistry.ts:609-633`的`binding.contractOperationId` | 已漂移的WebSocket identity helper；本任务禁止借机设计WS |
| `runtime-assembly-unary-dispatch.test.ts`对`contractOperationId`/`testEffectDoubles`的赋值 | canonical F359 reject mutation，应保留为负例 |
| `scripts/lib/package-service-ecosystem-smoke-oracle.mjs:17-22,318-337` | smoke receipt仍校验v1 identities及contract/operation，是test-runner receipt的下游旧consumer |
| `scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs:4-17,70-95` | 同一旧receipt的fixture data，随HTTP receipt consumer迁移 |
| `scripts/tests/package-service-ecosystem-smoke-real.test.mjs:248-284` | 仍mutation/assert WebSocket contract/operation receipt；完整WS test owner不作为HTTP正例 |
| `runtime_execution/tests/support.rs:5-10` | test-runner仍使用RuntimeAssembly v1 test constants；F359 consumer后继应机械切到v2 |
| `test-runner/tests/package_service_contract_deployment.rs:1261-1270` | 仍以entrypoint contract identity检查case确定性；改为deployment/key/identity |
| 同文件`:1693-1711,1750-1766` | 仍期望unary/WS contract及2-operation smoke contract；HTTP正例改为two gateway/zero-op，WS断言移出 |

assembly HTTP owned paths完成后的scoped反搜应为零：

```text
test-runner/src/package_test_assembly.rs
test-runner/src/runtime_execution.rs
router/src/router/assemblyControlPlane.ts

ContractOperationId|contract_operation_id|contractOperationId|testEffectDoubles
```

`ecosystem_smoke_fixture.rs`也应在HTTP/WS fixture拆分后对这些名字为零；general legacy和明确reject
mutation不计为残留。

## 4. Frozen synthetic gateway contract

### 4.1 共享builder

新增一个test-runner-owned canonical helper，输入exact private callable与return schema，只构造以下surface：

```json
{
  "protocol": {
    "kind": "http",
    "surface": {
      "adapterKind": "typedJson",
      "dispatchMode": "unary",
      "externalSources": [{ "kind": "http.body" }],
      "requestBodySchema": { "kind": "null" },
      "responseSchema": "<null-or-string>",
      "streamItemSchema": null
    }
  },
  "externalErrorProjection": {
    "kind": "fixed",
    "version": "v1"
  }
}
```

对应execution plan固定为：

```json
{
  "kind": "typedJson",
  "args": [
    {
      "param": "body",
      "source": { "kind": "http.body" }
    }
  ]
}
```

entry固定`pre: null`、`guard: null`。helper必须先证明handler是implementation symbol中的exact private
`InternalFunction`，signature精确覆盖`body`，再调用现有
`skiff_artifact_identity::gateway_entry_identity`；不得手拼digest、从selector派生identity或接受public
operation target。

### 4.2 Package-test exact entry

每个case冻结为：

| fact | exact value |
| --- | --- |
| selector | `http`, host `case-{index}.package-test.skiff.localhost`, method `POST`, path `/__skiff/package-test/{index}` |
| deployment-local key | `run` |
| mode | `unary` |
| adapter | 上述`typedJson`, `body <- http.body` |
| request body | UTF-8 bytes `null` |
| handler | generated private `fn <test>Gateway(body: null) -> null`；内部调用现有零参test body后返回`null` |
| response | HTTP 200；`content-type: application/json; charset=utf-8`；body bytes `null` |
| contract | 当前per-case service coordinate，`operations: {}`、`packageTypeRequirements: []` |
| operation bindings | `[]` |
| gateway entries / ingress | exactly 1 / exactly 1 |

该surface的canonical preimage和identity为：

```text
{"schema":"skiff-gateway-entry-identity-v1","surface":{"externalErrorProjection":{"kind":"fixed","version":"v1"},"protocol":{"kind":"http","surface":{"adapterKind":"typedJson","dispatchMode":"unary","externalSources":[{"kind":"http.body"}],"requestBodySchema":{"kind":"null"},"responseSchema":{"kind":"null"},"streamItemSchema":null}}}}

skiff-gateway-entry-v1:sha256:cfcfced94f984612809ce837f81e975016b09f206925389d95e925e087fc32d4
```

identity不含index、selector、key、param name、handler或package build，因此所有package-test case可合法共享该
protocol identity；exact owner仍由`deployment + key + identity`三元组区分。

### 4.3 Ecosystem exact entries

ecosystem HTTP projection总计两个entry：

1. 上述package-test `run` entry；
2. smoke deployment的`probe` entry：

| fact | exact value |
| --- | --- |
| selector | `http`, host `ecosystem-smoke.skiff.localhost`, method `POST`, path `/probe` |
| deployment-local key | `probe` |
| mode/adapter/request | `unary`; 同一typed-null adapter；body `null` |
| handler | exact private `main.__skiffHttpProbe(body: null) -> string` |
| response | HTTP 200 JSON string；例如marker `A`的body bytes为`"A"` |
| contract / operations | `test.skiff/ecosystem-smoke@1.0.0` zero-op contract；operation bindings `[]` |

normal smoke fixtures的private wrapper调用现有`main.marker()`；I02 wrapper调用当前API映射实际指向的
`main.submitSpawnReceipt()`。wrapper作为普通private source参与新production artifact编译且不进入
`api.yml`；fixture assembly不得在编译后改写或重签artifact，也不会把gateway handler公开成service API。
缺失或signature不符必须fail closed。

`Null -> String` surface的exact identity为：

```text
skiff-gateway-entry-v1:sha256:adfaa17c077af0388f2b5751bbe4b9ba392ec647f5ce33022c8e8ec83eaf6653
```

两个entry可共享同一builder及request adapter；不能共享同一个identity，因为response schema分别为Null与
String。

## 5. Frozen test control request/response

### 5.1 Control request

`POST /__skiff/test-dispatch`只接受下面的exact object。示例中的assembly、generation、selector、mode和
gateway identity都由test-runner已经发布并收到activation receipt的canonical facts提供：

```json
{
  "kind": "test",
  "routing": {
    "kind": "runtimeAssembly",
    "assemblyIdentity": "skiff-runtime-assembly-v2:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "assemblyGeneration": 7,
    "gatewayEntryIdentity": "skiff-gateway-entry-v1:sha256:cfcfced94f984612809ce837f81e975016b09f206925389d95e925e087fc32d4",
    "ingress": {
      "protocol": "http",
      "host": "case-0.package-test.skiff.localhost",
      "method": "POST",
      "path": "/__skiff/package-test/0"
    }
  },
  "mode": "unary",
  "httpRequest": {
    "method": "POST",
    "url": "http://case-0.package-test.skiff.localhost/__skiff/package-test/0",
    "path": "/__skiff/package-test/0",
    "query": [],
    "headers": [
      {
        "name": "content-type",
        "value": "application/json"
      }
    ]
  },
  "payloadBase64": "bnVsbA==",
  "timeoutMs": 30000
}
```

冻结规则：

- exact top-level fields就是示例中的六项；`contractOperationId`、`deployment`、`gatewayEntryKey`、
  `testEffectsEnabled`、`testEffectDoubles`及unknown fields全部拒绝。
- parser不lowercase host、不uppercase method、不补path、不重算identity。它只strict decode，然后把整个
  routing/mode/httpRequest与active snapshot exact binding逐值比较。
- `assemblyIdentity`来自published assembly ref，`assemblyGeneration`来自strict activation receipt，
  selector/mode/key/identity来自projected deployment/entrypoint。Router只验证，不猜测。
- `payloadBase64`必须是canonical standard Base64（decode后再encode逐值相等），package-test必须解出exact
  `null` bytes；`timeoutMs`必须是positive safe integer。
- control object没有wire flag。只有成功decode且exact match的`kind: "test"`分支调用private
  test-header builder；该builder复用F359 production header facts，最后把
  `testEffectsEnabled`设为true并再次跑canonical F359 validator。
- production `assemblyHttpRequestHeader`不得增加可由caller传入的boolean开关，仍固定false。

Router必须提供显式test-only dispatcher/registry入口，例如
`dispatchAssemblyTestBinary` + `pickAssemblyTestDispatchConnection`。普通`dispatchBinary`继续走
`validateDispatchRequest`并拒绝true；test-only入口要求header flag精确为true，但仍复用全部
assembly/generation/selector/identity/mode/http metadata校验。不要增加`skipValidation`或通用
`allowTestEffects` boolean。

### 5.2 Control success response

外层HTTP 200继续返回runtime canonical `response.end`和opaque payload，不把业务JSON解码进control schema：

```json
{
  "ok": true,
  "header": {
    "schemaVersion": "skiff-runtime-frame-v1",
    "type": "response.end",
    "requestId": "package-test-request-1",
    "payloadPresent": true,
    "httpResponse": {
      "status": 200,
      "headers": [
        {
          "name": "content-type",
          "value": "application/json; charset=utf-8"
        }
      ]
    }
  },
  "payloadBase64": "bnVsbA=="
}
```

test-runner必须strict decode exact fields，并以inner `header.httpResponse.status`判断case结果；仅看control
endpoint外层2xx不再算通过。package-test success还要求payload exact为`null`。runtime
`response.error`、inner non-2xx、malformed/empty request id或wrong type、`payloadPresent`不一致、invalid Base64或unknown
field都算case失败。Router parse/match/dispatch错误继续返回non-2xx control error，不能伪装为上述success。

### 5.3 Effect传递

```text
test source effect
  -> test_overlay hidden setup CompilerTestEffectRegister
  -> private HTTP wrapper calls existing test body
  -> test body first calls setup
  -> kind:test Router branch emits F359 testEffectsEnabled=true
  -> F365 Host preserves flag, initializes an internal empty double context
  -> runtime creates test-enabled interpreter
  -> inline setup registers exact effect sequences in that interpreter
```

wire JSON上不再存在`testEffectDoubles`。production HTTP仍走同一Host/eval seam，但header为false，执行
`CompilerTestEffectRegister`时继续fail closed。

## 6. 后继文件边界与最小DAG

```text
R0 Router isolated test-dispatch checkpoint
  -> T1 package-test gateway producer + strict control consumer
       -> F381 real Registry runtime cases rerun
       -> T2 ecosystem HTTP fixture/receipt convergence

T1与T2由同一test-runner owner串行；不得并行写共享fixture/helper/tests。
```

### R0 Router shared checkpoint

写入：

- `router/src/router/assemblyControlPlane.ts`
- `router/src/router/assemblyHttpGateway.ts`（只复用canonical header facts；production false不变）
- `router/src/router/assemblyRuntimeRegistry.ts`
- `router/src/router/runtimeDispatcher.ts`
- `router/tests/assembly-runtime-endpoint.test.ts`
- `router/tests/runtime-assembly-unary-dispatch.test.ts`
- 必要的direct dispatcher/registry test helper

禁止扩入：

- `router/src/protocol/**`、snapshot/loader DTO；
- Rust transport/Host/eval；
- general legacy `router/src/router/controlPlane.ts`；
- WebSocket identity、connect/receive/message协议。

### T1 package-test consumer

写入：

- 新的小型test-only canonical gateway helper module；
- `test-runner/src/test_overlay.rs`
- `test-runner/src/package_test_assembly.rs`
- `test-runner/src/runtime_execution.rs`
- `test-runner/src/runtime_execution/wire.rs`
- `test-runner/src/runtime_execution/tests/**`
- `test-runner/tests/package_service_contract_deployment.rs`

不改`runtime/package-test/src`；F361已经冻结deployment/key/identity exact lookup。

### T2 ecosystem HTTP consumer

写入：

- `test-runner/src/ecosystem_smoke_fixture.rs`
- `test-runner/src/bin/package_service_smoke_fixture.rs`
- 四个现有ecosystem fixture的private HTTP wrapper及其direct tests
- `scripts/lib/package-service-ecosystem-smoke-oracle.mjs`
- `scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs`
- 对应non-live Node receipt tests

T1/T2共享gateway helper、`package_test_assembly.rs`产物、
`package_service_contract_deployment.rs`和receipt shape，所以应由同一owner串行提交。R0与T1没有文件重叠，
但control JSON是producer/consumer接口，必须先冻结R0正负测试再接T1。

完整WebSocket smoke不得混入T2。HTTP receipt应只声称两个真实可路由HTTP entry；现有v1 receipt的
contract/operation/三个entrypoint shape是breaking旧合同，后继固定升级为
`skiff-package-service-smoke-fixture-v2`，不得dual-read。

## 7. 后继测试矩阵与命令

### 7.1 R0 Router

正例：

- exact `kind: test` body命中active selector/key-derived identity并发出F359 header；
- header的assembly/generation/identity/mode/httpRequest逐值等于control request和snapshot；
- test-only dispatch wire flag为true，opaque`null` bytes不变；
- canonical `response.end` header/payload逐字节返回；
- production HTTP builder和ordinary dispatcher继续false。

负例：

- old `contractOperationId`、`testEffectDoubles`、control `testEffectsEnabled`和unknown field；
- wrong/missing kind、v1 assembly、stale generation、wrong gateway identity/mode/selector；
- lower/upper-case修复才能命中的host/method、mismatched URL/path、invalid Base64、zero/unsafe timeout；
- 通过ordinary dispatcher发送true仍报
  `active RuntimeAssembly dispatch rejects test effect controls`；
- test-only dispatcher收到false或legacy header失败。

命令：

```bash
cd /Users/geek/workspace/<router-successor-worktree>
pnpm --filter @skiff/router exec vitest run \
  tests/assembly-runtime-endpoint.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  tests/assembly-replica-dispatch.test.ts
pnpm --filter @skiff/router exec tsc --noEmit --pretty false
git diff --check
```

`tsc`结果必须证明R0-owned HTTP/control production与direct tests为零错误；若integration仍保留已归档的
WebSocket-only错误，逐项记录而不得为追求全绿扩入WS设计。

### 7.2 T1 package-test

正例：

- 每case zero-op contract、one gateway/one ingress、empty operation bindings；
- exact selector/key、fixed `Null -> Null` identity、private wrapper和reference closure；
- base assembly service/package bindings与per-case state isolation保持；
- emitted control body exact等于§5.1，strict response decoder验证inner status/body；
- inline setup effect在true flag下实际生效。

负例：

- missing/wrong-signature/public handler、wrong key/identity、orphan gateway、duplicate selector；
- contract重新出现operation、deployment重新出现operation binding；
- wrapper未调用test body或test body未先调用setup；
- response outer 200但inner 500、wrong header、invalid Base64或non-null success body；
- scoped old-field反搜非零即失败。

命令：

```bash
cd /Users/geek/workspace/<test-runner-successor-worktree>
cargo test --locked -p skiff-test-runner --lib runtime_execution -- --test-threads=1
cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment -- --test-threads=1
cargo check --locked -p skiff-test-runner --bins
cargo clippy --locked -p skiff-test-runner --all-targets --no-deps -- -D warnings
git diff --check
```

T1通过后，从F381 clean checkpoint重新运行：

```bash
cd /Users/geek/workspace/<skiff-packages-f381-worktree>
npm run test:registry
```

必须观察8个runtime cases真实执行，不能只接受fixture assembly成功。

### 7.3 T2 ecosystem HTTP

正例：

- assembly有两个root、两个zero-op contract、两个deployment、两个HTTP gateway ingress；
- package-test identity为
  `cfcfced94f984612809ce837f81e975016b09f206925389d95e925e087fc32d4`；
- `/probe` identity为
  `adfaa17c077af0388f2b5751bbe4b9ba392ec647f5ce33022c8e8ec83eaf6653`；
- normal marker和I02 submit wrapper分别返回原有最终结果；
- HTTP receipt只保存deployment/key/identity/mode/selector，不保存contract/operation；
- fixture assembly前后使用同一个新编译production PackageArtifact ref；source新增private wrapper会正常产生
  新build identity，但fixture不得在编译后改写或重签artifact。

负例：

- 把optional websocket算作第三个HTTP gateway或恢复旧WS operation ingress；
- 两个不同response schema错误共享identity；
- wrapper进入public API、handler指向public operation target、missing wrapper silent fallback；
- old receipt schema dual-read或把不可路由WS entry继续标成candidate entrypoint。

命令：

```bash
cd /Users/geek/workspace/<test-runner-successor-worktree>
cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment \
  ecosystem_http_fixture_uses_two_gateway_entries_without_ws_compat -- --exact
cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment \
  i02_submit_probe_is_private_http_gateway_not_service_operation -- --exact
node --test scripts/tests/package-service-ecosystem-http-fixture.test.mjs
git diff --check
```

最后一个是后继新增的v2 HTTP receipt direct test。现有
`package-service-ecosystem-smoke-{real,lifecycle}.test.mjs`仍拥有完整WebSocket流程，不属于T2通过证据；
真实WebSocket、stable instance和完整ecosystem live smoke同样不在本HTTP后继内。

## 8. 用户决策

不需要用户决策。后继应按`R0 -> T1 -> F381 rerun`优先恢复Registry动态测试，再由同一test-runner owner
完成T2。若实现发现必须修改F359字段集、F365 Host admission、gateway identity preimage或WebSocket协议，
那是新的shared scope expansion，不能在本冻结合同下自行扩大。
