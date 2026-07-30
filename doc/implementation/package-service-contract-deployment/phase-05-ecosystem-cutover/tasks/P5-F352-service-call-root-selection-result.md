# P5-F352 Explicit service-call root selection result

状态：Completed（C1 shared Package/Service checkpoint，含late closure；未运行workspace/root、
stable/live，未push）。

## 1. Exact checkpoint

| 项目 | commit | tree |
| --- | --- | --- |
| 本leaf base | `acbb4d7ea1174289c9c89c93b866dd1511815e17` | `e21f0cca314e408890631e1f8c09f6b34a4ed5b9` |
| initial production/tests | `dd13c0aaa5565d4ecaae55dfe3e759186f568a7b` | `ce5276f0f39d7c2406b97b6a52b4941bc905e5e7` |
| initial result | `8bfd73b88fc8d47d1a92fed59c3a6fb9aa893ef8` | `6e1d07cef940efa5a169170b9558acf2c7152f60` |
| late closure production/tests | `f2e5b6daa08a5ec261ca2374dc737d3d8996cb3f` | `fd032f9714243f687fbed0d903cae28b17e67ab9` |

工作分支为`codex/p5-f352-service-call-selection`，worktree为
`/Users/geek/workspace/skiff-p5-f352-service-call-selection`。没有merge/rebase integration，
没有操作stable instance或live配置。

## 2. Canonical api.yml与service-root admission

`compiler/input`现在是唯一`api.yml` parser owner：

- `api.yml`必须存在、非空且root为mapping；无公开API只能写top-level`{}`；
- 任意nested empty mapping都会拒绝，避免`functions: {}`或mixed empty namespace绕过；
- string function leaf保持unmarked；
- object function leaf严格只接受`source + serviceCall: true`，缺字段、`false`、非boolean、
  duplicate或unknown field全部拒绝；
- public instance继续要求`const + interfaces`，并可选严格`serviceCall: true`；
- duplicate source/parser已从`compiler/source`删除，prelude loading复用canonical input owner。

Marker经过以下typed chain保真，不从raw YAML直接构造artifact root：

```text
PublicationApiSpec
  -> PublicationApi / PublicCallable / PublicInstance
  -> ExportBindingModel
  -> compiled ProjectionInput
  -> PackageExports
  -> PackageArtifact.serviceCallRoots
```

普通package pipeline若看到marker会列出全部路径并拒绝。真实service与compiler-owned test
service都必须携带由`read_service_package_root`构造的opaque `ServicePackageRoot`，不能再用
ambient bool或裸`serviceId`授权；pipeline还会验证该root的`PackageManifest`与实际compile
input完全一致。Marker解析到type/interface/const时在typed publication boundary拒绝。

## 3. PackageArtifact typed roots与identity

新增closed、strict、无default的`PackageServiceCallRoot`：

- `Function { publicPath, callableId }`
- `PublicInstance { publicPath, methods: BTreeMap<method, callableId> }`

Public instance methods来自其显式listed interface method surface；普通impl helper不会进入root。
Strict artifact validation要求：

- root path非空且唯一；
- 每个public callable ID必须严格为
  `pkg-callable:<packageId>:<publicPath>`，不能通过同步伪造root/link map改名；
- function path、`PackageCallableId`、public symbol与`PublicFunction` link kind精确一致；
- `implementationLinks.functions`与`implementationLinks.implMethods`的执行坐标集合必须互斥，
  且分别与全部public function/method link精确闭合，function不能重标为method，反之亦然；
- public instance必须有非空interfaces，instance identity与public path一致；
- instance root method map必须与Local ABI公开method map完全一致；
- 每个`<instance>.<method>` path、callable ID与`ImplMethod` link kind精确一致；
- receiver public/source const必须落到同一exact record declaration；非泛型receiver使用bare
  nominal，泛型receiver使用同一ordered binder构造的`AppliedNominal`，具体const参数arity与
  closedness也必须精确；
- interface必须解析到当前package的exact interface declaration；generic interface arguments
  从source conformance穿过compiled/projection handoff，并在比较method surface前完成替换；
- public/source alias twin比较完整ordered interface signature，而不只比较method name；
- 每个method的public callable、interface signature和implementation executable必须在参数名、
  参数类型、返回类型、`maySuspend`、receiver encoding、file/index/symbol/kind上完全一致；
- public callable `typeParams`必须精确等于receiver ordered binders；implementation executable
  的`typeParams`与`self`类型也必须使用同一scope。native/provider/method-level generic等
  unsupported surface fail closed；
- 同一callable不能被多个root重复选择。

Identity generation变化：

| domain | leaf base | selector checkpoint | late closure |
| --- | --- | --- | --- |
| PackageArtifact schema | `skiff-package-artifact-v5` | `...-v6` | `...-v7` |
| build projection marker | `skiff-package-artifact-build-identity-v4` | `...-v5` | `...-v6` |
| Package build ID prefix | `skiff-package-build-v6:sha256` | `...-v7:sha256` | `...-v8:sha256` |
| Local ABI marker | `skiff-package-artifact-local-abi-identity-v3` | unchanged | `...-v4` |
| Local ABI prefix | `skiff-package-local-abi-v5:sha256` | unchanged | `...-v6:sha256` |

Roots按public path规范排序后进入Package build preimage；不进入Local ABI preimage。测试证明root
重排identity稳定、root变化改变build identity、相同public surface的Local ABI identity不变。
Late closure新增required `PackageCallableSignature.typeParams`，因此它同时进入Local ABI与build
preimage；旧wire缺少`serviceCallRoots`/`typeParams`或使用旧schema/identity generation都会
fail closed。Official std的`std.actor.find`保留ordered `["T", "Id"]` binder；依赖identity
binding也不再丢失该scope。

## 4. ServiceContract projection

Service API不再把全部boundary-available callable自动公开为operation：

- 只有typed explicit roots生成operations；
- unmarked Available与Unavailable callable都只留在Package API visibility；
- marked unavailable roots以一个
  `UnavailableServiceCallRoots { path -> [all structured reasons] }`错误聚合报告；
- public instance只展开listed interface methods；
- schema/type transitive closure只从selected Available operations开始；
- visibility仍列出全部public callables，只有selected Available项带
  `serviceOperationId`，human receipt把其余项显示为`package-only`；
- zero marker的compiler projection生成合法、稳定的zero-operation `ServiceContract`与
  `ServiceProtocolIdentity`，且不能附带不可达的package type requirements；
- standalone `compile_service_contract_definition`继续要求至少一个operation，只有typed
  package-to-service projection拥有zero-operation authority。

新增真实端到端fixture覆盖marked function、marked public instance两个listed methods、
unmarked function，以及改变root selection后的Package build、Local ABI和Service protocol
identity矩阵。

## 5. 验收矩阵

| 验收项 | 证据 | 结果 |
| --- | --- | --- |
| selector非零 | input 12；contract 3；artifact identity 8 | PASS |
| ordinary package marker拒绝 | `pipeline::tests::service_call_marker_is_rejected_for_an_ordinary_package_root` | PASS |
| missing/blank拒绝，`{}`成功 | canonical parser tests | PASS |
| string unmarked/object marked | parser tests + real E2E | PASS |
| false/unknown/duplicate/missing field拒绝 | parser negative matrix | PASS |
| marker只解析function/public instance | source non-function marker negative test | PASS |
| typed function/instance roots | real `service_call_roots` E2E | PASS |
| public instance全部且仅listed methods | E2E + strict tamper tests | PASS |
| marked unavailable聚合全部原因 | contract projection test | PASS |
| unmarked Available/Unavailable不进contract | contract selection/visibility test | PASS |
| zero operation与稳定identity | contract + artifact-identity tests + E2E | PASS |
| selected-only schema closure | unmarked Available返回package schema但contract requirements为空 | PASS |
| root reorder/build/Local ABI identity matrix | artifact identity tests | PASS |
| root selection改变Service protocol | real E2E | PASS |
|旧PackageArtifact wire拒绝 | required field、stale schema/build tests | PASS |
| forbidden production scope | 无gateway/ingress、runtime/router生产语义或lockfile修改；runtime/deployment仅机械补strict fixture字段 | PASS |

## 6. 执行结果

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-compiler-input api_yml -- --list` | PASS；12 tests，非零 |
| `cargo test -p skiff-compiler-contract service_call -- --list` | PASS；3 tests，非零 |
| `cargo test -p skiff-artifact-identity package_artifact -- --list` | PASS；8 tests，非零 |
| `cargo test -p skiff-compiler-input api_yml` | PASS；12 passed |
| `cargo test -p skiff-compiler-input` | PASS；82 passed |
| `cargo test -p skiff-compiler-source --lib` | PASS；311 passed |
| `cargo test -p skiff-compiler-contract` | PASS；5 passed |
| `cargo test -p skiff-artifact-model package_artifact` | PASS；5 passed |
| `cargo test -p skiff-artifact-identity package_artifact` | PASS；8 passed |
| `cargo test -p skiff-artifact-identity --lib` | PASS；104 passed |
| `cargo test -p skiff-compiler-projection package_artifact` | PASS；50 passed |
| `cargo test -p skiff-compiler --lib service_call` | PASS；1 passed |
| `cargo test -p skiff-compiler --test service_call_roots service_call` | PASS；1 passed |
| `cargo test -p skiff-compiler --test generated_service_deployment` | PASS；8 passed |
| `cargo test -p skiff-compiler --bin skiff-compiler human_service_api` | PASS；2 passed |
| `node --test scripts/tests/package-service-authoring.test.mjs` | PASS；9 passed |
| scoped eight-crate `cargo check` | PASS；仅既有warnings |
| `cargo test -p skiff-deployment --no-run` | PASS |
| `git diff --check` | PASS |

Late closure复验：

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-artifact-identity public_instance --lib` | PASS；6 passed |
| `cargo test -p skiff-artifact-identity --lib` | PASS；112 passed |
| `cargo test -p skiff-compiler-projection --lib` | PASS；50 passed |
| `cargo test -p skiff-compiler-input` | PASS；82 passed |
| six-package scoped `cargo test --no-fail-fast` | PASS；artifact model/identity、projection input/projection、source、contract全绿 |
| `cargo test -p skiff-compiler --lib service_call` | PASS；1 passed |
| `cargo test -p skiff-compiler --test service_call_roots` | PASS；2 passed，含generic `Worker<T>` |
| `cargo test -p skiff-compiler --bin skiff-compiler human_service_api` | PASS；2 passed |
| `node --test scripts/tests/package-service-authoring.test.mjs` | PASS；9 passed |
| `node scripts/check-artifact-identity-single-source.mjs --self-test` | PASS |
| changed Rust `rustfmt --check`与`git diff --check` | PASS |

在隔离worktree把late closure叠到F353 checkpoint
`f129bc7a8d18fef8d7ec6fca587e6332fd73cd3d`，并为F353-only test literal机械补required
`typeParams`后，以下共享核心路径通过：

- projection `package_schema`：11 passed；
- `cargo test -p skiff-compiler service_call`与完整`service_call_roots`：PASS，后者2 passed；
- official std deterministic authoring：PASS；v8 golden为
  `skiff-package-build-v8:sha256:eb7a294930e76caabe86d73600f84da63cdb4e88ac8c763bbb64377ffe7ea69f`。

Initial checkpoint时，Task要求的`cargo test -p skiff-compiler service_call`会先编译该package
声明的全部integration targets，因此在进入F352 selector前被当时F353拥有的
`compiler/tests/std_package_imports.rs`阻断：

1. `ConcreteNominal`已没有`type_arguments`字段；
2. `TypeRefIr::AppliedNominal` match arm缺失。

F352的library selector与独立真实E2E target均通过；late closure没有修改schema generic
eligibility admission，只补全callable applied-nominal owner identity。

Initial checkpoint的scoped `cargo fmt ... -- --check`报告两个base格式漂移：

1. `compiler/driver/authoring/package_publication/tests.rs:49`
2. `compiler/tests/websocket_ingress.rs:9`

第二项属于明确禁止的ingress scope；late closure保留第一个文件的既有两行格式并排除该
base drift，其余changed Rust文件逐文件`rustfmt --check`，`git diff --check`通过。

## 7. 明确残余

1. F353 checkpoint `f129bc7`仍有其owner范围内的旧fixture/integration债务：新增的
   `PackageCallableSignature` literals缺`typeParams`、部分package fixture缺required
   `api.yml`、lazy std closure测试没有resolved `std` owner，以及service-conformance fixture
   缺database/package requirement。组合验收未把这些问题扩进F352。
2. Full `check-artifact-identity-single-source.mjs`在本leaf的旧base上报告六项随后已在main修复的
   single-source债务；脚本self-test通过，本leaf没有复制这些无关mainline修复。
3. Zero-operation projected contract已经闭合；deployment是否允许空`operationBindings`仍由
   deployment/gateway owner决定。
4. Generic interface、type declaration binder name/duplicate的更广泛语言级规范化，以及
   F353完整schema eligibility矩阵，继续由generic schema owner负责；F352只校验PublicInstance
   所消费的exact instantiated surface。

未运行workspace/root、stable/live；未修改gateway/ingress DTO、runtime/router生产语义或
lockfile；runtime/deployment改动仅为strict artifact test fixture字段；未push。
