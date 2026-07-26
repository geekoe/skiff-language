# P5-F352 Explicit service-call root selection result

状态：Completed（C1 shared Package/Service checkpoint；未运行workspace/root、stable/live，未push）。

## 1. Exact checkpoint

| 项目 | commit | tree |
| --- | --- | --- |
| 本leaf base | `acbb4d7ea1174289c9c89c93b866dd1511815e17` | `e21f0cca314e408890631e1f8c09f6b34a4ed5b9` |
| production/tests | `dd13c0aaa5565d4ecaae55dfe3e759186f568a7b` | `ce5276f0f39d7c2406b97b6a52b4941bc905e5e7` |

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

普通package pipeline若看到marker会列出全部路径并拒绝。真实service pipeline显式授权root；
compiler-owned test-service authority仍可使用同一package编译阶段，避免test service被误判为普通
package。Marker解析到type/interface/const时在typed publication boundary拒绝。

## 3. PackageArtifact typed roots与identity

新增closed、strict、无default的`PackageServiceCallRoot`：

- `Function { publicPath, callableId }`
- `PublicInstance { publicPath, methods: BTreeMap<method, callableId> }`

Public instance methods来自其显式listed interface method surface；普通impl helper不会进入root。
Strict artifact validation要求：

- root path非空且唯一；
- function path、`PackageCallableId`、public symbol与`PublicFunction` link kind精确一致；
- public instance必须有非空interfaces，instance identity与public path一致；
- instance root method map必须与Local ABI公开method map完全一致；
- 每个`<instance>.<method>` path、callable ID与`ImplMethod` link kind精确一致；
- 同一callable不能被多个root重复选择。

Identity generation变化：

| domain | before | after |
| --- | --- | --- |
| PackageArtifact schema | `skiff-package-artifact-v5` | `skiff-package-artifact-v6` |
| build projection marker | `skiff-package-artifact-build-identity-v4` | `...-v5` |
| Package build ID prefix | `skiff-package-build-v6:sha256` | `...-v7:sha256` |
| Local ABI marker/prefix | unchanged | unchanged |

Roots按public path规范排序后进入Package build preimage；不进入Local ABI preimage。测试证明root
重排identity稳定、root变化改变build identity、相同public surface的Local ABI identity不变。
旧wire缺少`serviceCallRoots`或使用旧schema/build generation会fail closed。

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
- zero marker生成合法、稳定的zero-operation `ServiceContract`与
  `ServiceProtocolIdentity`。

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
| forbidden production scope | 无gateway、ingress、generic eligibility、runtime、router、test-runner、lockfile修改 | PASS |

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

Task要求的`cargo test -p skiff-compiler service_call`会先编译该package声明的全部integration
targets，因此在进入F352 selector前被base已有、F353拥有的
`compiler/tests/std_package_imports.rs`阻断：

1. `ConcreteNominal`已没有`type_arguments`字段；
2. `TypeRefIr::AppliedNominal` match arm缺失。

F352的library selector与独立真实E2E target均通过，没有修改generic eligibility owner。

Task指定的scoped `cargo fmt ... -- --check`只报告两个未修改的base格式漂移：

1. `compiler/driver/authoring/package_publication/tests.rs:49`
2. `compiler/tests/websocket_ingress.rs:9`

第二项属于明确禁止的ingress scope；本leaf所有changed Rust文件均已逐文件rustfmt，
`git diff --check`通过。

## 7. 明确残余

1. Strict required `serviceCallRoots`使16个禁止修改的runtime test/fixture
   `PackageArtifact` literals需要后续机械补`Vec::new()`；本leaf没有越界修改runtime。
2. Strict required `api.yml`使runtime/live/test-runner中既有missing/blank fixture需要由其owner
   改为`{}`；本leaf只更新允许范围内的直接compiler fixtures。
3. Zero-operation ServiceContract identity已经合法，但deployment generation当前仍要求非空
   `operationBindings`。Deployment gateway/ingress语义不属本leaf，后续owner需决定zero-operation
   deployment行为。
4. `compiler/driver/authoring/package_publication/tests.rs`仍固定旧std build ID；F353 generic
   eligibility修复后需按v7 build domain重算golden。两个script smoke oracle也仍有base已有的旧
   generation债务。
5. `cargo test -p skiff-compiler-emission --no-run`被base已有
   `compiler/emission/src/emission/package_artifact/tests/requirements.rs`缺少`CallIr.site`阻断；
   与service-call root selection无关。

未运行workspace/root、stable/live；未修改gateway/ingress DTO、generic eligibility、
runtime/router/test-runner、lockfile；未push。
