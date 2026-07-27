# P5-F440A External manifest三仓owner审计结果

状态：**PASS / TASK_EXECUTABLE**。本节点只完成owner审计，没有实现、迁移或运行live/stable。

## 1. 结论与输入

三个不可执行条件均未出现：

- 权威设计明确规定deployment timeout只来自`config.<profile>.yml`，`service.yml.timeout`必须删除并
  fail closed；
- 权威设计明确规定PackageArtifact不读取`service.yml`、`http.yml`或`websocket.yml`；
- `http.yml`顶层direct mapping、`websocket.yml`的`path/connect/jsonRpc`及JSON-RPC adapter source已经
  在`c2c1c41c36bce9945d617a8bd8e0eea834f5478d`冻结，不存在待本任务补设计的公共语义。

当前实现仍把HTTP、WebSocket和一个不被policy使用的`timeout`字段放在
`ServiceManifestAuthoring`中，这是需要hard cut的实现债，不是设计阻塞。新增external manifest会扩展
artifact DTO vocabulary；schema marker/version、producer和所有reader只能由下文单一shared owner一起改，
不能由fixture migration自行决定。

审计读取的固定快照如下：

| Repo | 审计root | Commit | Tree | 起始状态 |
| --- | --- | --- | --- | --- |
| Skiff | 本任务worktree | `a5fdcbd712dbcd30f6a421ee48b6b2876f970e36` | `33911300aa666a610f6ed82087682efe1153fe97` | clean |
| Internals | `/Users/geek/workspace/internals-phase-05-integration` | `faa11b188c570ca763f107ddd829d52b8fe8861f` | `140d3a03851b64d513fd97c5860e713b8fc314de` | clean |
| skiff-packages | `/Users/geek/workspace/skiff-packages-phase-05-integration` | `f8c634ce4573506e35f6bc1c7cc1e4eef9992a78` | `eb00877ef260d122552af1ff0491c74102adbd57` | clean |

提交前复核时，三个clean integration head分别为：

- Skiff `75117f615d36b89bef01e851cadd6fa15b859a92` /
  `c0f77d3793107ef1605d90449e03164ea1620b20`；
- Internals `605ebd209dacac7c95aa79dc3a508d428a352453` /
  `95cc84051c350f45e38a6092958d58734c5278db`；
- skiff-packages仍为表中commit/tree。

Skiff从固定输入前进的diff只含取消hard cut、builtin canonical spelling和后继调度文档；Internals diff只含
F440D的Agine Host peer protocol fixture/type/test，没有改`agine/service`或本结果列出的manifest owner。
因此本结果仍以任务worktree固定的`a5fdcbd7`及表中另外两个commit为审计输入，不把并行集成提交混入本leaf。

总计发现24个tracked service root：Skiff 13、Internals 4、skiff-packages 7。其中8个有HTTP
surface，5个有WebSocket surface（Agine同时有两者），12个为零external ingress。

## 2. Authoring读取与typed projection owner

### 2.1 当前精确owner

| 层 | 文件 / 符号 | 当前事实 | target责任 |
| --- | --- | --- | --- |
| strict DTO | `artifact-model/src/ecosystem_authoring.rs`：`ServiceManifestAuthoring`、`HttpGatewayEntryAuthoring`、`WebSocketConnectAuthoring`、`WebSocketGatewayEntryAuthoring`、`deserialize_http_gateway_entry_map` | `ServiceManifestAuthoring`当前拥有`id/kind/serviceCalls/http/websocket/timeout`；DTO均`deny_unknown_fields`，HTTP map有duplicate-key visitor | `ServiceManifestAuthoring`只保留`id/kind/serviceCalls`；HTTP map和WebSocket singleton成为独立strict document DTO；新增strict JSON-RPC entry DTO和duplicate method/key校验 |
| DTO export | `artifact-model/src/lib.rs` | re-export当前四个authoring DTO | re-export拆分后的document DTO；不提供旧inline alias |
| YAML reader | `compiler/input/src/service_config.rs`：`SERVICE_CONFIG_FILE`、`ServicePackageRoot`、`read_service_package_root`、`read_service_manifest`、`validate_http_authoring`、`validate_websocket_authoring`、`read_config_profiles`、`config_profile_name` | 只打开`service.yml`和`config.*.yml`；HTTP/WS从`ServiceManifestAuthoring`内取；`http.yml`/`websocket.yml`目前完全不读 | 增加`HTTP_CONFIG_FILE`/`WEBSOCKET_CONFIG_FILE`及独立reader；`ServicePackageRoot`分别保存service/http/websocket；external file可选，但出现时必须同时存在合法`package.yml`、`api.yml`、`service.yml` |
| source root classifier | `scripts/skiff.mjs`：`detectRootKind` | 有`package.yml`就认作package；只有`service.yml`时报service-only；会静默接受`package.yml + http.yml` | 检查`http.yml`/`websocket.yml`；external-only或package+external但无service必须在Cargo前失败 |
| watch registry classifier | `scripts/skiff-dev-sync.mjs`：`classifyAuthoringRoot`、`normalizedRoots` | 只枚举`package.yml/contract.yml/deployment.yml`，不知道external files | 保持package为唯一root kind，同时验证external⇔service约束和retired独立authoring |
| compiler root branch | `compiler/driver/authoring.rs`：`build_package_after_platform_context_guard`、`reject_legacy_service_authoring` | `root.join("service.yml").is_file()`决定是否生成contract/deployment；external-only会被忽略；只拒绝`contract.yml/deployment.yml` | root inventory先fail closed，再由typed `ServicePackageRoot`驱动；ordinary Package不得带external manifests |
| compiler package input | `compiler/driver/input/source_graph.rs`：`PackageSourceInput`；`compiler/driver/input/compile_input.rs`：`PackageCompileInput` | 明确只含package manifest/source graph/resources/test overlay，不含service或route | 保持不变；严禁给它增加external manifest字段 |
| service projection | `compiler/driver/pipeline/mod.rs`：`compile_service_package` | 先`compile_package(input)`，再只用`service.id/service_calls`调用`project_service_api` | 保持顺序和输入边界；这是PackageArtifact、ServiceContract不受external manifest影响的第一道证明 |
| HTTP typed projection | `compiler/driver/http_gateway_projection/mod.rs`：`project_http_gateway`、`ProjectedHttpGateway`；`resolver::ExactCallableResolver`；`schema::ExactTypeClassifier` | 接收整个`ServiceManifestAuthoring`并读`.http` | 只接收独立typed HTTP document；继续从精确PackageArtifact closure解析callable和linked type |
| WebSocket typed projection | `compiler/driver/websocket_gateway_projection.rs`：`project_websocket_gateway`、`ProjectedWebSocketGateway` | 接收整个service，只有path/connect，无JSON-RPC | 接收独立typed WebSocket document；connect与每个`jsonRpc` method分别投影，method entry只能unary且必须恰好一个`websocket.jsonRpcParams` |
| deployment producer | `compiler/driver/generated_deployment.rs`：`GeneratedServiceDeploymentInput`、`generate_service_deployment`、`generated_revision`、`deployment_policy` | 输入只有`service`和profile；HTTP/WS从service读；policy timeout只从profile读 | input显式分开`service/http/websocket`；所有三份已校验typed authoring进入deployment revision，policy仍只读profile |

目标读取规则只有一处：`compiler/input/src/service_config.rs`在source-root trust boundary读取三份YAML。
`compiler/driver/**`、`artifact-identity/**`、Router和Runtime只消费typed DTO或生成artifact，绝不能再次读原始
YAML。`service.yml`继续由`read_service_manifest`读取；`http.yml`和`websocket.yml`由同模块新增的独立
reader读取；下游projection接收三个不同字段。

### 2.2 Artifact DTO与reader follower

External manifest会产生新的typed gateway vocabulary，以下是同一公共schema的直接consumer：

- `artifact-model/src/gateway.rs`
  - `GatewayAdapterKind`当前只有`TypedJson/RawHttp/WebSocketConnect`；
  - `GatewayAdapterSource`当前没有`WebSocketJsonRpcParams`或`WebSocketBusinessIdentity`；
  - `adapter_source_is_allowed`当前只覆盖HTTP/connect；
  - `GatewayProtocolSurface`当前只有`Http/WebSocketConnect`；
  - `GatewayEntryProtocolSurface`、`GatewayExternalSchema`及WebSocket shape常量是公共typed surface owner。
- `artifact-model/src/deployment.rs`
  - `IngressProtocol`、`IngressSelector`、`DeploymentIngressBinding`；
  - `GatewayAdapterPlan`、`DeploymentGatewayEntry`；
  - `ServiceDeploymentInput`、`ServiceDeployment`。
- `artifact-model/src/schema.rs`
  - `SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION`和`SERVICE_DEPLOYMENT_SCHEMA_VERSION`。
- `artifact-identity/src/gateway.rs`
  - `normalize_gateway_entry_protocol_surface`、
    `validate_gateway_entry_protocol_surface`、`gateway_entry_identity_projection`；
  - `GatewayEntryIdentityProjection`的private字段保证selector/key/handler/build不能混入preimage。
- `artifact-identity/src/deployment/validation.rs`
  - `validate_gateway_entries`、`validate_ingress_bindings`及protocol/adapter plan对应关系。
- `deployment/src/projection/mod.rs`
  - `project_service_deployment`把path-free typed input解析成canonical deployment并赋identity。
- `router/src/router/runtimeAssemblyDeploymentSnapshot.ts`
  - `joinRuntimeAssemblyDeployments`、`decodeServiceDeployment`、
    `decodeDeploymentGatewayEntry`、`decodeWebSocketDeploymentGatewayEntry`、
    `deriveWebSocketEntryId`、`decodeExternalSource`、`decodeDeploymentPolicy`。
- `router/src/router/runtimeAssemblySnapshot.ts`
  - `decodeRuntimeAssemblyIngressSelector`、`decodeGatewayIngressDeclaration`及assembly ingress index。
- `runtime/loader/src/runtime_assembly/gateway_ingress.rs`
  - `HydratedGatewayEntry`、`HydratedGatewayIngress`、`hydrate_gateway_ingress`。
- `runtime/host/src/loader/assembly_admission.rs`
  - 当前在`IngressProtocol`与`GatewayProtocolSurface`上只有HTTP/connect两个match arm。
- `runtime/host/src/loader/active_assembly_context.rs`、
  `runtime/request/src/http_gateway_target.rs`、
  `runtime/request/src/websocket_connect_target.rs`
  - 当前均假定connect是唯一WebSocket execution target。

Router/Runtime的JSON-RPC broker和实际request execution不属于external-manifest leaf自行实现；它们必须由
F440B结果产生的shared RPC checkpoint拥有。这里列出的artifact reader/admission seam必须与该checkpoint
共享同一个schema输入，不能让manifest迁移leaf复制一套临时JSON-RPC DTO。

## 3. Identity、revision与cache边界

| 域 | canonical owner / preimage | external manifest变化 |
| --- | --- | --- |
| Package local ABI | `artifact-identity/src/package_artifact.rs`：`PackageArtifactLocalAbiIdentityProjection`、`package_artifact_local_abi_identity_projection` | 不变；preimage只有package id和public symbols |
| Package build | 同文件及`package_artifact/projection.rs`：`PackageArtifactBuildIdentityProjection`、`package_artifact_build_identity_projection` | 不变；preimage来自source/File IR、implementation symbols/links、package schema、requirements、resources、semantic facts等，没有service selection或external manifest |
| Service protocol | `artifact-identity/src/contract.rs`：`ServiceProtocolIdentityProjection`、`service_protocol_identity_projection`、`service_protocol_identity` | 不变；preimage只有service id、已选择operations和reachable package type requirements |
| Gateway entry | `artifact-identity/src/gateway.rs`：`GatewayEntryIdentityProjection`、`canonical_gateway_entry_identity_bytes`、`gateway_entry_identity` | protocol-visible shape/source/schema变化时改变；仅host/method/path/key/handler/build变化时不改变，因为这些不是protocol preimage |
| WebSocket entry id | 同文件：`WebSocketEntryIdProjection`、`websocket_entry_id` | 只由service id和compiler-owned WebSocket entry key决定；path或JSON-RPC method不进入 |
| Deployment revision | `compiler/driver/generated_deployment.rs::generated_revision` | 当前hash `(service.id, profile_name, package_build_id, profile, full inline service)`；target必须改为canonical `(service identity/selection, optional typed http, optional typed websocket, profile_name, package_build_id, profile)`，不再存在service timeout双owner |
| Deployment artifact | `artifact-identity/src/deployment.rs`：`DeploymentArtifactIdentityProjection`、`service_deployment_identity_projection`、`service_deployment_identity` | 改变；preimage含revision、gateway entries、ingress、implementation/bindings/config/policy等 |
| Runtime assembly | `artifact-identity/src/runtime_assembly.rs`：`AssemblyIdentityProjection`、`runtime_assembly_identity_projection` | 改变；deployment refs、activation templates和gateway ingress都在preimage |
| watch fingerprint | `scripts/skiff-dev-sync.mjs::rootsFingerprint/hashTree` | 已对root中除`.git/build/node_modules/target`外的全部regular file做path+bytes SHA-256；新增或修改external file已会触发重建，无需新增文件名枚举 |
| canonical source provenance | Internals `scripts/canonical-source-provenance.mjs::resolveCanonicalSourceProvenance/readGitProvenance` | repo commit/tree会改变；这是workflow provenance，不是artifact identity。它只从`package.yml/service.yml`读coordinate，external文件没有id，不应伪造coordinate读取 |
| runtime binary source key | `scripts/lib/source-key.mjs::sourceKeyFromInputs`及`build-runtime-stack.mjs`调用点 | 输入只覆盖runtime stack源码，不是service authoring cache；不要把external manifest加进这个无关cache |
| package source archive | `scripts/lib/package-source-archive.mjs::collectPackageSourceArchivePaths` | 应继续只归档`package.yml`、`.skiff`和package-declared resources；把external manifests加入会违反PackageArtifact不读取它们的边界 |

必须新增两组mutation证据，不能用一句“hash变了”代替：

1. **selector-only vector**：只改`http.yml`的host/method/path或`websocket.yml`的path/method，序列化后的
   PackageArtifact、ServiceContract和GatewayEntryIdentity bit-identical，但deployment revision、
   DeploymentArtifactIdentity和RuntimeAssemblyIdentity改变。
2. **protocol-shape vector**：只把external entry切到另一个已存在、不同params/result shape的private
   handler（package source本身不变），PackageArtifact和ServiceContract bit-identical，
   GatewayEntryIdentity、deployment和assembly改变。若换成同shape handler，GatewayEntryIdentity允许不变，
   但handler binding和revision仍使deployment改变。

现有
`compiler/tests/http_gateway_projection.rs::selector_body_shape_implementation_and_adapter_plan_obey_identity_boundaries`
和
`compiler/tests/websocket_ingress.rs::path_only_and_connect_variants_preserve_protocol_identity_boundaries`
已经覆盖大部分域，但它们从inline DTO构造；shared checkpoint必须把证据改为真实
`service.yml + http.yml/websocket.yml`文件mutation。现有
`package_artifact_build_v10_preimage_excludes_service_selection`继续保留。

## 4. Control-file discovery、watch、copy与temporary root

| Owner | 当前硬编码 / 行为 | 必须动作 |
| --- | --- | --- |
| `scripts/skiff.mjs::detectRootKind` | 只枚举`package.yml/service.yml` | 加external-only与package-with-external-without-service负例 |
| `scripts/skiff-dev-sync.mjs::classifyAuthoringRoot` | 只枚举`package.yml/contract.yml/deployment.yml` | 加external/service invariant；`rootsFingerprint/hashTree`保持全树hash |
| `compiler/driver/authoring.rs` | 仅按`service.yml`存在决定service branch | 在任何package build前reject非法external inventory |
| `compiler/input/src/resources.rs::is_skiff_control_file` | 未列`http.yml/websocket.yml` | 加两者；否则可把控制文件声明成Package resource并污染PackageArtifact |
| `scripts/lib/publication-resources.mjs::controlFilePatterns` | 未列两者 | 加两者并扩充`check-publication-resource-archive.mjs`负例 |
| `scripts/lib/publication-resources.mjs::visitManifestDirectories` | 只访问`package.yml/service.yml` | 这是“可声明resources的manifest”walk；external DTO禁止`resources`，因此不应为了发现resource而访问external文件，但要用测试把该意图固定 |
| `scripts/lib/package-source-archive.mjs` | 只收package manifest/source/resource | 明确保留；增加“external control files不进入package archive且不能被resource引用”测试 |
| `test-runner/src/canonical_package.rs::read_test_service_profile` | 只用`service.yml`判断是否有test service，然后调用`read_service_package_root` | 角色判断保持；typed root reader自动读external files。新增非法external-only和合法split root测试 |
| `test-runner/src/package_service_host_fixture.rs::copy_fixture_tree` | 递归复制整个root | 已会复制新文件；新增copy receipt断言，不改成文件白名单 |
| `test-runner/tests/package_service_contract_deployment.rs::copy_tree` | 递归复制整个root | 同上 |
| `scripts/lib/package-service-host-negative-probe.mjs` | `cp(..., recursive: true)`并对整个目录做SHA-256 snapshot | 已会复制/证明external files；更新expected snapshot测试即可 |
| `scripts/skiff-dev-sync.mjs::runDevSyncOnce` | source root原地传compiler，只临时写`assembly.yml` | 不复制service文件；临时assembly owner不变 |
| `router/tests/helpers/compilerArtifacts.ts::writeCompilerGeneratedFixtureArtifactRoot` | 从checked-in compiler fixture动态构建，临时只写assembly | fixture拆分即可；helper无需复制白名单 |
| Internals `scripts/canonical-source-provenance.mjs` | service coordinate只读`package.yml/service.yml` | 保持coordinate owner；combined owner补“optional external文件不参与coordinate”测试 |
| Internals四个service receipt test | 都从`service.yml`切出/解析HTTP，Agine还解析旧WS | 分别改读`http.yml`/`websocket.yml`；不能继续用字符串slice伪装parser兼容 |
| skiff-packages `scripts/test-packages.mjs::assertPackageExists` | 只用`service.yml`确认test role | 零ingress test roots可保持；补普通package带external file的terminal负例应在Skiff classifier，而不是这里复制parser |

`service.yml`文件名本身仍是合法role marker，因此“只检查service.yml”不一律是bug。必须修改的是会因此
漏读、忽略或归档external control file的owner；只读取service id/kind的consumer应保持窄职责。

## 5. 三仓service-root矩阵

记号：`G/P`为guard/pre；`U/S`为unary/server-stream数；`—`表示无该surface。

### 5.1 Skiff（13）

| Root | 当前HTTP | 当前WebSocket | timeout当前 → target | target external文件 / direct owner |
| --- | --- | --- | --- | --- |
| `compiler/tests/fixtures/router-websocket-fixture` | inline named map，1 `typedJson`，G/P=`0/0`，U/S=`1/0` | — | `config.dev.yml: 120000`，保持 | 新`http.yml`放`ping`；`service.yml`只留id。`router/tests/compilerGeneratedManifestCompatibility.test.ts`、`router/tests/helpers/compilerArtifacts.ts` |
| `runtime/encrypted-storage-live/default-service` | legacy `http.routes` 21，未写kind；实际signature均为`HttpRequest -> HttpResponse`，target为21 `rawHttp`；global guard、无pre，U/S=`21/0` | — | `service.yml timeout.default:120000` → `config.dev.yml timeout:120000` | 该root还缺`package.yml/api.yml`，必须一起canonicalize；新`http.yml`逐entry复制`internal.live.guard`并去掉`root.` selector。`scripts/lib/encrypted-storage-live-harness.mjs`、`scripts/check-db-encrypted-storage-live.mjs` |
| `runtime/encrypted-storage-live/mapped-service` | legacy routes 13；实际均`rawHttp`；global guard、无pre，U/S=`13/0` | — | inline `120000` → `config.dev.yml` | 同上；旧`packages`移回`package.yml`，新`http.yml`。同一managed encrypted live harness/check |
| `runtime/live-tests` | legacy routes 6；实际均`rawHttp`，G/P=`0/0`，U/S=`5/1`（`streamEcho`） | — | inline `120000` → canonical selected profile（当前live命令通常为`config.runtime-live.yml`；若继续允许任意environment，workflow必须先证明对应tracked profile存在） | 还缺`package.yml/api.yml`；旧`packages`移回package；新`http.yml`。`scripts/lib/verify-live-plan.mjs`与`verify*.test.mjs` |
| `test-runner/fixtures/alias-return-catch-once-tests` | — | — | `config.skiff-test.yml:30000`，保持 | 不新增external文件；`scripts/tests/skiff-source-test-suite.test.mjs` |
| `test-runner/fixtures/package-service-host/consumer-tests` | — | — | tracked `config.skiff-test.yml:30000` | 不新增；Host fixture/test-runner combined |
| `test-runner/fixtures/package-service-host/consumer` | — | — | 无tracked profile；`prepare_service_root`生成`config.<env>.yml:1000` | 不新增；`test-runner/src/package_service_host_fixture.rs` |
| `test-runner/fixtures/package-service-host/provider` | — | — | 同上生成`1000` | 不新增；serviceCalls保持；Host fixture combined |
| `test-runner/fixtures/package-service-i02-spawn-submit` | — | inline singleton path`/socket` + connect；无legacy receive/jsonRpc | `config.skiff-test.yml:30000` | 新`websocket.yml`；service只留id/kind。`package_service_contract_deployment.rs`、`package-service-i02-combined.test.mjs` |
| `test-runner/fixtures/package-service-websocket-generation-a` | — | `/socket` + connect；无receive/jsonRpc | `config.skiff-test.yml:30000` | 新`websocket.yml`；`package_service_contract_deployment.rs`、ecosystem HTTP fixture test |
| `test-runner/fixtures/package-service-websocket-generation-b` | — | 同上 | `config.skiff-test.yml:30000` | 同上 |
| `test-runner/fixtures/package-service-websocket-smoke` | — | 同上 | `config.skiff-test.yml:30000` | 同上 |
| `test-services/std` | — | — | `config.skiff-test.yml:30000` | 不新增；std test runner/source-suite receipts |

Legacy encrypted/runtime-live三个root的“kind未写”是当前文件事实；target kind由canonical test/live harness
owner决定。无论选择ordinary service还是`kind:test`，external manifest和timeout owner都不能回退到旧格式。

### 5.2 Internals（4）

| Root | 当前HTTP | 当前WebSocket | timeout当前 → target | target external文件 / direct receipt |
| --- | --- | --- | --- | --- |
| `agine/service` | inline named map 36 `rawHttp`，G/P=`0/0`，U/S=`36/0` | legacy `routes: [{path:/ws, operation:websocket}]`；无connect、无declared jsonRpc | inline `default:120000` **且** `config.dev.yml:120000` → 只保留config | 新`http.yml`含36 entry；新`websocket.yml`保留`path:/ws`并使用F440B/F440D冻结的declared `jsonRpc` mapping，绝不能搬旧routes。`service-api-receipt.{mjs,test.mjs}`、`internal/agine_service_architecture.test.mjs` |
| `aihub/service` | 7 `rawHttp`，G/P=`0/0`，U/S=`5/2`；两个events handler为stream | — | inline `default:120000` **且** config `120000` → 只保留config | 新`http.yml`；service保留id及2个serviceCalls。`service-api-receipt.{mjs,test.mjs}` |
| `codex-relay/service` | 30 `rawHttp`，G/P=`0/0`，U/S=`27/3`；三个`proxy_runtime.proxy`为stream | — | 已仅`config.dev.yml:120000` | 新`http.yml`；service保留id/`relayProxy`。`service-api-receipt.test.mjs`中的manifest parser及generated-record receipt |
| `skiff-platform/account` | 21 `rawHttp`，G/P=`0/0`，U/S=`21/0` | — | 已仅`config.dev.yml:120000` | 新`http.yml`；service只留id。`service-api-receipt.{mjs,test.mjs}`及21-entry generated-record receipt |

Agine leaf被typed RPC checkpoint遮挡：HTTP部分虽然可机械拆分，但同一service root只允许一个writer，故该
writer必须等WebSocket JSON-RPC entry key/method/handler已经由F440D和F440B shared schema确定后一次完成，
不能先提交一个仍含legacy WS或临时删除`/ws`的半迁移。

### 5.3 skiff-packages（7）

| Root | 当前HTTP / WS | timeout | target与direct receipt |
| --- | --- | --- | --- |
| `registry` | `0 / 0`，serviceCalls 20 | `config.dev.yml:30000` | 不新增external文件；`scripts/registry-service-source.test.mjs`和`registry-service-receipt.test.mjs`继续证明0 gateway |
| `tests/aliyunoss` | `0 / 0`，kind test | `config.skiff-test.yml:30000` | 不新增；`scripts/test-packages.mjs` |
| `tests/http-session` | `0 / 0`，kind test | `30000` | 同上 |
| `tests/openai-live` | `0 / 0`，kind test | `60000` | 同上 |
| `tests/openai` | `0 / 0`，kind test | `30000` | 同上 |
| `tests/registry` | `0 / 0`，kind test | `30000` | 同上 |
| `tests/track` | `0 / 0`，kind test | `30000` | 同上 |

Official packages没有要移动的inline ingress。它们仍需要独立migration/verification node，因为strict DTO
hard cut后必须证明所有零ingress root可构建、没有被工具误要求创建空external文件，并在artifact schema
version变化时刷新Registry receipt。

## 6. Fixture、golden与fail-closed清单

### 6.1 必须改写的inline authoring

| Owner | inline/fixture点 | 动作 |
| --- | --- | --- |
| `artifact-model/src/ecosystem_authoring.rs` tests | `service_manifest_decodes_named_http_entries_*`、HTTP duplicate/unknown、WebSocket positive/legacy/duplicate tests | 改为三个独立document DTO测试；新增service inline`http/websocket/timeout`拒绝 |
| `compiler/input/src/service_config.rs` tests | `reads_service_as_package_root_and_profiles`以及全部HTTP/WS positive/negative通过`read_service_yml`写inline字段 | helper分别写`service.yml/http.yml/websocket.yml`；保留serviceCalls和package-owned-field负例 |
| `compiler/driver/generated_deployment.rs` tests | path-only WS与legacy operation inline parse | 改测独立WebSocket DTO；legacy shape仍拒绝 |
| `compiler/tests/http_gateway_projection.rs` | `parse_service(http)`、temp `service.yml`与HTTP strings | fixture input拆成service+HTTP DTO/文件；identity mutation改用真实external file |
| `compiler/tests/websocket_ingress.rs` | `parse_service(fields)`、`connect_authoring` | 同上，增加JSON-RPC method投影/负例 |
| `compiler/tests/generated_service_deployment.rs` | HTTP/WS inline DTO、`GeneratedServiceDeploymentInput` helper | 显式传`http/websocket`；timeout测试仍只改profile |
| `compiler/tests/service_calls_manifest_selection.rs`、`service_conformance.rs` | 只写id/serviceCalls | 保持service-only；用作“无external文件合法”证据 |
| `runtime/eval/src/runtime_http_gateway/tests.rs::write_service_fixture` | temp service中inline HTTP | 写独立`http.yml` |
| `runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs::write_service_fixture` | 同上 | 写独立`http.yml` |
| `test-runner/tests/package_service_contract_deployment.rs` | `test_service_environment_profile_*`只写id/kind；四个checked-in WS fixture读取`.service.websocket` | 前者保持；后者读`ServicePackageRoot.websocket`并刷新generated identities |
| `scripts/tests/package-service-authoring.test.mjs` | temp service只含id/serviceCalls | 保持；另加split build案例 |
| `scripts/tests/package-service-dev-sync.test.mjs` | temp `service.yml`与root classifier | 加合法external及非法external-only矩阵 |
| `scripts/tests/skiff-test-cli.test.mjs` | optional service-role与service-only负例 | 扩成`http.yml`、`websocket.yml`单独/组合负例 |
| `scripts/tests/package-service-host-negative-probe.test.mjs` | temp kind:test service和recursive copy | service本身保持；copy snapshot加入external文件向量 |
| `scripts/tests/package-service-i02-combined.test.mjs` | exact inline WS `service.yml`字符串 | 分别断言精简service和exact `websocket.yml` |
| `scripts/tests/package-service-ecosystem-http-fixture.test.mjs` | 四个WS fixture exact service字符串 | 同上 |
| checked-in root | 第5节所有有inline ingress的12个root | 新文件hard cut；不得dual-read |

`router/tests/config.test.ts`中的`missing-service.yml/invalid-service.yml`是Router自身配置fixture名，不是Skiff
service authoring，不应被字符串替换误伤。

### 6.2 Golden与generation常量

- `cross-system-fixtures/package-service-ecosystem/checkpoint.json`当前错误声明
  `"service.yml": ["id","http","websocket","timeout"]`；必须改成service的`id/kind?/serviceCalls`，
  并增加`http.yml`、`websocket.yml`、`config.<profile>.yml.timeout` owner。
  `verify.mjs`同步更新。
- `test-runner/tests/package_service_contract_deployment.rs`第2082–2115行的四组：
  - `skiff-package-build-v10`与`skiff-package-local-abi-v7`常量必须**保留原值**，这是manifest split不污染
    PackageArtifact的golden；
  - `skiff-deployment-artifact-v2`和`skiff-runtime-assembly-v2`必须从新producer重新生成并刷新；
  - 现有connect-only
    `d3288437...` GatewayEntryIdentity在connect protocol surface/schema marker未改变时应保留。若shared owner
    有意升级整个gateway identity schema marker，则由M0/M1一次刷新，fixture leaf不得局部改常量。
- `compiler/tests/websocket_ingress.rs`的connect gateway identity `d3288437...`遵守同一规则。
- test-runner内`run/probe` HTTP gateway identities
  `cfcfced9...` / `adfaa17c...`不是source manifest生成的WS entry，split不应刷新。
- `router/tests/helpers/compilerArtifacts.ts`动态编译record，没有checked-in artifact hash；只需拆fixture。
- `scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs`中的重复字符fake identities是wire
  validator fixture，不是generation golden；manifest split本身不刷新。
- `cross-system-fixtures/package-service-ecosystem/runtime-*-wire.json`中的synthetic/fixed identities属于
  transport corpus；只有F440B shared RPC schema改变对应wire时才由其owner刷新，不由manifest migration
  顺手改。
- Internals generated-record validators大多使用prefix regex和entry closure，不应引入手写新hash；
  canonical build receipt是证据。

### 6.3 必须保留/新增的terminal负例

必须保留并迁移到正确reader的旧拒绝：

- `service.yml`中的`http`、`websocket`、`timeout`；
- HTTP的`routes`、`entries`、global guard/pre、`operation`、`handlerArgs`、unknown/duplicate/missing fields；
- WebSocket的null/scalar/list/multi-map、`routes`、`operation`、`receive`、`message`、`context`、author id、
  null connect和duplicate fields；
- `contract.yml/deployment.yml`独立authoring；
- service-only root和manifest-less root。

新增负例：

- ordinary package或package+external但无`service.yml`；
- null/scalar/list `http.yml`，以及empty/null/scalar/list `websocket.yml`；同时用正例固定
  `http.yml: {}`和WebSocket path-only合法；
- external file的wrapper key（`http:`/`websocket:`）；
- duplicate JSON-RPC key/method/handler、空method、`$/`保留method；
- JSON-RPC raw receive/notification/event fallback、transport id、手写schema；
- JSON-RPC handler缺少或重复`websocket.jsonRpcParams`，stream return，阶段错误source，
  `businessIdentity`类型错误；
- `http.yml`/`websocket.yml`被声明为publication resource；
- stale deployment/artifact中JSON-RPC adapter kind、protocol surface、source阶段不一致。

历史task/result只作为调度证据，不是schema owner；本实现DAG不得改写任何历史result。

## 7. 冻结实现DAG与互斥写集

```text
F440A result
  -> M0 strict DTO/root reader + compiler typed projection
      -> M1 deployment/artifact/identity follower
      -> S0 classifier/control-file/test-runner discovery
      -> S1 Skiff canonical fixtures + inline helpers/goldens
      -> S2 Skiff legacy live roots/harness migration
      -> IA2 AIHub migration
      -> IC2 codex-relay migration
      -> IK2 account migration
      -> P0 official packages zero-ingress migration

F440B result + M0 + M1 + required cancellation checkpoint
  -> R0 Router/Runtime typed JSON-RPC artifact reader/admission checkpoint

F440D + M0 + M1 + R0
  -> IA1 Agine one-shot HTTP/WebSocket service-root migration

M0 + M1 + S0 + S1 + S2 + IA1 + IA2 + IC2 + IK2 + P0 + R0
  -> C0 single three-repo focused combined owner
```

互斥写集如下；未列路径默认为禁止写：

| Node | 唯一写集 | 明确不拥有 |
| --- | --- | --- |
| **M0** | `artifact-model/src/ecosystem_authoring.rs`、`gateway.rs`、`deployment.rs`、`schema.rs`、`lib.rs`；`compiler/input/src/service_config.rs`及export；`compiler/driver/{authoring.rs,generated_deployment.rs,http_gateway_projection/**,websocket_gateway_projection.rs,pipeline/mod.rs,lib.rs}`；相关`compiler/tests/{http_gateway_projection,websocket_ingress,generated_service_deployment}.rs` | artifact identity、Router/Runtime execution、任何真实service root |
| **M1** | `artifact-identity/src/{gateway.rs,deployment.rs,deployment/**,tests/**,constants.rs,lib.rs}`相关部分；`deployment/**`；`scripts/check-artifact-identity-single-source.mjs`及其self-test | authoring parser、Router broker、fixtures |
| **R0** | `router/src/router/runtimeAssembly{Deployment,}Snapshot.ts`及直接loader tests；`runtime/loader/src/runtime_assembly/gateway_ingress.rs`；`runtime/host/src/loader/{assembly_admission.rs,active_assembly_context.rs}`；F440B冻结的JSON-RPC target/execution/transport文件 | YAML、service source、HTTP migrations；实际broker文件以F440B result最终清单为准，不能双写 |
| **S0** | `scripts/skiff.mjs`、`scripts/skiff-dev-sync.mjs`、`scripts/lib/publication-resources.mjs`、`scripts/check-publication-resource-archive.mjs`及对应CLI/dev-sync tests；`compiler/input/src/resources.rs`；`test-runner/src/canonical_package.rs` | checked-in service fixtures |
| **S1** | `compiler/tests/fixtures/router-websocket-fixture/**`；`test-runner/fixtures/**`、`test-services/std/**`、`test-runner/tests/package_service_contract_deployment.rs`、`test-runner/src/package_service_host_fixture.rs`；两个runtime temp fixture writer；`scripts/tests/package-service-{i02-combined,ecosystem-http-fixture,host-negative-probe}.test.mjs`及其recursive-copy helper | legacy live roots、cross-system checkpoint、parser |
| **S2** | `runtime/encrypted-storage-live/{default-service,mapped-service}/**`、`runtime/live-tests/**`；`scripts/lib/encrypted-storage-live-harness.mjs`、`scripts/check-db-encrypted-storage-live.mjs`、`scripts/lib/verify-live-plan.mjs`及直接verify tests | stable instance、live execution、其它fixtures |
| **IA1** | `agine/service/{service.yml,http.yml,websocket.yml,config.dev.yml,service-api-receipt.mjs,service-api-receipt.test.mjs}`和`agine/service/internal/agine_service_architecture.test.mjs`中manifest/receipt assertions | shared Internals workflow、其它services、Host protocol（F440D owner） |
| **IA2** | `aihub/service/{service.yml,http.yml,config.dev.yml,service-api-receipt.mjs,service-api-receipt.test.mjs}` | Agine/codex/account |
| **IC2** | `codex-relay/service/{service.yml,http.yml,config.dev.yml,service-api-receipt.test.mjs}` | 其它services |
| **IK2** | `skiff-platform/account/{service.yml,http.yml,config.dev.yml,service-api-receipt.mjs,service-api-receipt.test.mjs}` | 其它services |
| **P0** | skiff-packages的`registry/service.yml`、`tests/*/service.yml`及对应config（预计内容不变）；`scripts/{registry-service-source.test.mjs,registry-service-receipt.test.mjs,test-packages.mjs}` | Skiff/Internals |
| **C0** | Skiff `cross-system-fixtures/package-service-ecosystem/{checkpoint.json,verify.mjs}`；Internals共享`canonical-source-provenance.mjs`、`prepare-canonical-assembly.mjs`及其test（仅确有必要时）；combined receipt/result | 任一前置节点的production/service root；不得借combined修叶子 |

M0与M1可在同一shared branch连续实现，但仍按上述文件owner分提交。S1不能先于M0/M1刷新hash，因为那会把
旧producer输出固化为新golden。S2不运行managed live；live只在最终独立验收/唯一final gate。

## 8. 后继验证矩阵

| Node | 首个failing test | Focused selector | Reverse search | identity / receipt证据 | 遮挡 |
| --- | --- | --- | --- | --- | --- |
| M0 | 新增`service_manifest_rejects_inline_external_fields`和`reads_split_external_manifests`；当前前者接受inline、后者没有DTO/reader | `cargo test -p skiff-artifact-model ecosystem_authoring`; `cargo test -p skiff-compiler-input service_config`; `cargo test -p skiff-compiler --test http_gateway_projection --test websocket_ingress --test generated_service_deployment` | `rg -n 'service\\.(http|websocket)|ServiceManifestAuthoring.*(http|websocket|timeout)' artifact-model compiler` | 两组真实文件mutation，PackageArtifact/Contract exact bytes相等，gateway/deployment/assembly按第3节变化 | 无；是shared根 |
| M1 | 新JSON-RPC protocol/source反序列化或identity validation当前无variant/match arm | `cargo test -p skiff-artifact-identity gateway`; `cargo test -p skiff-artifact-identity deployment`; `cargo test -p skiff-deployment` | exhaustively搜索`GatewayProtocolSurface::WebSocketConnect`、`GatewayAdapterKind::WebSocketConnect`、`GatewayAdapterSource::WebSocket` | selector-only与shape-change gateway identity；tampered deployment fail closed | 等M0 schema |
| R0 | Router loader对`websocketJsonRpc`/新source报invalid；Runtime admission无match arm | `pnpm --dir router test -- tests/filesystem-runtime-assembly-snapshot-loader.test.ts tests/compilerGeneratedManifestCompatibility.test.ts`; `cargo test -p skiff-runtime-loader gateway_ingress`; `cargo test -p skiff-runtime-host assembly_admission` | Router/Runtime内搜索旧三种adapter source/kind与两种protocol的closed union | compiler生成artifact从Router到Runtime admission exact key/identity/source；stale/tampered拒绝 | 等M0/M1、F440B result和取消checkpoint；broker行为另由F440B |
| S0 | external-only root当前被`detectRootKind/classifyAuthoringRoot`漏过；resource validator当前接受`http.yml` | `node --test scripts/tests/package-service-dev-sync.test.mjs scripts/tests/skiff-test-cli.test.mjs`; `cargo test -p skiff-compiler-input resources`; `node scripts/check-publication-resource-archive.mjs` | `rg -n \"service\\\\.yml|package\\\\.yml\" scripts/skiff.mjs scripts/skiff-dev-sync.mjs scripts/lib test-runner/src/canonical_package.rs`逐项归类 | watcher改external bytes必触发一次build；package archive和PackageBuildId不变 | reader案例等M0 |
| S1 | exact fixture tests仍期望`service.yml.websocket`；test-runner generated deployment/assembly constants变旧 | `cargo test -p skiff-test-runner --test package_service_contract_deployment ecosystem_http_private_wrappers_compile_for_all_owned_source_fixtures`; `node --test scripts/tests/package-service-i02-combined.test.mjs scripts/tests/package-service-ecosystem-http-fixture.test.mjs`；对应runtime fixture unit test | `rg -n '^[[:space:]]*(http|websocket|timeout):' --glob service.yml compiler/tests/fixtures test-runner test-services`应为空 | 四组PackageBuild/ABI常量原值，deployment/assembly新值；recursive copy前后包含external文件 | 等M0/M1；JSON-RPC execution不在此node |
| S2 | 真实三个root当前没有`package.yml`且仍是legacy route shape；`verify-live-plan`已明确报terminal migration | `node --test scripts/tests/verify.test.mjs scripts/tests/verify-live-registry.test.mjs scripts/tests/verify-live-plan-platform-source.test.mjs`；encrypted harness只做non-live plan/unit validation | 搜索三个root中的`version:|packages:|routes:|timeout:\\n  default`及`root.internal` selector | canonical build receipt为40个HTTP ingress；PackageArtifact不因route文件变化；deployment/assembly为新ref | 等M0/S0/M1；managed Mongo/runtime live留final gate |
| IA2 | `aihub/service/service-api-receipt.test.mjs`当前断言service含`http/timeout` | `node --test aihub/service/service-api-receipt.test.mjs` | root内`service.yml`不得匹配`http|websocket|timeout`，HTTP entry count=7 | migration前后PackageBuildId/ServiceProtocolIdentity exact相等；新deployment receipt为7 gateways、U/S=`5/2` | 等M0/M1 |
| IC2 | parser当前从service找`http:` | `node --test codex-relay/service/service-api-receipt.test.mjs` | 同上；http.yml entry/selector unique=30 | Package/Contract exact相等；generated record 30 gateways、27 unary/3 stream | 等M0/M1 |
| IK2 | 21-entry parser当前要求`lines[1] == 'http:'` | `node --test skiff-platform/account/service-api-receipt.test.mjs` | 同上，count=21 | Package/zero-operation Contract exact相等；21 closed raw gateways | 等M0/M1 |
| IA1 | 当前Agine direct tests明确接受legacy `websocket.routes/operation`；新parser应首先拒绝 | `node --test agine/service/service-api-receipt.test.mjs agine/service/internal/agine_service_architecture.test.mjs`，再跑F440D Host protocol focused tests | service无inline字段；http.yml=36；websocket.yml无`routes|operation|receive|message`且declared method/key唯一 | zero serviceCalls Contract保持零operation；36 HTTP receipts + declared JSON-RPC gateway receipts；PackageArtifact不受manifest变化 | 等F440D、M0/M1、R0；这是最强遮挡 |
| P0 | 新strict reader后的零ingress build/receipt；若无schema bump可能没有内容diff，禁止伪造空文件 | `node --test scripts/registry-service-source.test.mjs scripts/registry-service-receipt.test.mjs`; package test selector | `find registry tests -name http.yml -o -name websocket.yml`应为空；service只含id/kind/serviceCalls | Registry gatewayEntries=0；Package/Contract/Deployment receipt按shared schema决定是否bit-identical | 等M0/M1 |
| C0 | 任一canonical source仍inline、receipt closure不一致或checkpoint字段旧 | `node cross-system-fixtures/package-service-ecosystem/verify.mjs`; `node --test scripts/prepare-canonical-assembly.test.mjs`; 三仓canonical build到临时artifact root | 三仓全局`service.yml` inline字段、legacy routes、external-without-service、漏掉的`service.yml` parser sweep | 汇总每个服务Package/Contract不变证据、每个HTTP/WS gateway count、全部deployment/assembly新ref及repo provenance | 等所有叶子；唯一focused combined，昂贵final gate仍后置 |

每个migration leaf必须先保存baseline receipt（或引用已有canonical record），再改文件。Agine当前legacy
WebSocket shape已经不能通过strict compiler，因此若没有可用旧deployment baseline，不得伪造一个；它使用
M0共享mutation proof，加迁移后zero-operation Contract/Package receipt和完整gateway closure作为证据。

## 9. Combined验收不变量

最终C0至少执行以下静态不变量：

1. 每个`http.yml`/`websocket.yml`的父目录同时有regular
   `package.yml/api.yml/service.yml`；ordinary package无external文件。
2. 所有tracked `service.yml`只出现`id`、可选`kind`、可选`serviceCalls`；无
   `http/websocket/timeout/version/packages/services`。
3. HTTP-bearing root恰好8个、总entry 135：
   Skiff strict/legacy迁移后`1 + 21 + 13 + 6`，Internals`36 + 7 + 30 + 21`。
   Server-stream恰好6：runtime-live 1、AIHub 2、codex-relay 3。
4. WebSocket-bearing root恰好5；四个Skiff test fixture仍为`/socket + connect`且无JSON-RPC；
   Agine为`/ws`加唯一权威declared JSON-RPC map，无raw receive。
5. inline timeout清零；Agine/AIHub去掉重复owner；所有实际deployment timeout来自选中profile。
6. external-only mutation不改变PackageArtifact/ServiceContract；selector-only和shape-change向量分别证明
   downstream identity边界。
7. root watcher能发现新增/删除/修改external文件；Package source archive不包含它们；control files不能
   被声明为resource。
8. 所有legacy rejection仍terminal，不存在dual-read、compat alias或历史result改写。

本审计未授权push、stable watch注册、stable/live写入或昂贵gate。

## 10. 本leaf验证

- 对三个固定commit分别用tracked tree枚举`(^|/)service\.yml$`，结果为`13 + 4 + 7 = 24`，
  与第5节逐项一一对应，没有靠目录名推断root。
- 对每个有ingress的root逐文件复算map/route entry、adapter kind、guard/pre、stream signature和timeout；
  汇总为HTTP `8 roots / 135 entries / 6 server-stream`、WebSocket `5 roots`。
- 逐项reverse-search `service.yml`、`http.yml`、`websocket.yml`、control-file pattern、
  `ServiceManifestAuthoring`、gateway protocol/adapter union和递归copy helper，结果已归入第2、4、6节。
- 提交前重查三个integration worktree均clean；并行head相对固定输入没有manifest-owner漂移。
- `git diff --check`通过；本leaf未运行production/live测试，因为任务是只读owner审计，唯一变化是本结果。
