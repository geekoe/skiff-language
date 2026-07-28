# P5-F445H-I7-S1 Host/runtime/Router cross-layer receipt result

状态：

```text
PASS
S1_COMPLETE = YES
BLOCKING_ISSUES = 0
J_S1_PREREQUISITE_SATISFIED = YES
```

S1 现在从 S0 checked-in current-scope source 经同一个 canonical producer生成 exact artifact
closure，并由 Host filesystem resolver真实 admit、由 Router filesystem reader真实 load/join。
Router再使用读取出的 exact unary/server-stream binding经过真实 gateway/dispatcher/runtime socket
harness到达可观察HTTP status、headers、ordered chunks/body与单一terminal。没有 synthetic
deployment、Rust-generated replacement source或artifact identity rewrite参与这条receipt。

本节点只解除J的S1前置；不完成J、L0、L1或整个I7。

## 1. Parent chain and exact identity

直接父节点：

- `P5-F445H-I7R-cross-boundary-readiness-preflight-result.md`的冻结S1边界；
- `P5-F445H-I7-S0-real-source-artifact-checkpoint-result.md`：
  `S0_COMPLETE = YES / S1_UNBLOCKED = YES`；
- `P5-F445H-I6K-R4-independent-current-scope-reacceptance-result.md`：
  final I6 acceptance。

| 项 | 值 |
| --- | --- |
| baseline commit/tree | `6b5b71014800e4b18bc8ec70400510185e856fd6` / `dc6b9d5a2438d885770e074243368acde54cbcca` |
| task commit/tree | `faccddb26ab56539580e1d144be0c1e5bb148977` / `054cfa46c02c2a76e8746089911bc7fac02d0aad` |
| implementation commit/tree | `7ad9115e827f2f6380230ca13be6bff5dd856f32` / `db7911265cd008af77789648eb475fe5e7ea6d8b` |
| branch | `codex/p5-f445h-i7-s1-host-router` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-s1-host-router` |
| integration owner | `/root/phase05_integration_steward` |

最终result commit/tree在Git handoff中报告；result不能自引用自己的commit identity。

Exact S0 tuple保持不变：

```text
package      skiff-package-build-v10:sha256:9b03476e93f5ccb66dc69ff899f4a8fb9c68593e70c5aeda94d4e865aab688ad
contract     skiff-service-protocol-v5:sha256:9ea7ac440bd594ef31632c1c1914b40f2e92957e7fb0f73f587f4cb4d8563fa5
deployment   skiff-deployment-artifact-v3:sha256:aa74be018958d2e2375b91e500e4f73b6fea8fb97c4d694962d6745fe475791c
HTTP unary   skiff-gateway-entry-v2:sha256:0fd289d7eec4e03b01e9e8f5633aedd7e1cc64158fa7932f99a9686e559c02f2
HTTP stream  skiff-gateway-entry-v2:sha256:1aef41f397b7c817110cb0cc74a7b472ba9732c5ac6bcfe6e219e3ac51ab6bd0
WebSocket    skiff-gateway-entry-v2:sha256:f385624021966bab998385e1fd2c88804b51992f15f9c9d76c05d3e17a75018d
assembly     skiff-runtime-assembly-v2:sha256:ec66d8a209e65198ee5b82086a365a4b3a98021ef8117e2572c66fee8eac5f6e
```

## 2. Actual write set

Task/result：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-S1-host-runtime-router-cross-layer-receipt.md
  P5-F445H-I7-S1-host-runtime-router-cross-layer-receipt-result.md
```

Implementation：

```text
runtime/host/src/host/router_session/tests/runtime_assembly_request.rs
runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs
router/tests/helpers/compilerArtifacts.ts
router/tests/compilerGeneratedManifestCompatibility.test.ts
router/tests/assembly-http-gateway-stream.test.ts
router/tests/runtime-assembly-unary-dispatch.test.ts
```

没有production、protocol/schema/identity、compiler/deployment、S0 source、Internals、
`skiff-packages`、Cargo manifest/lockfile或F442 corpus写入。F442不需刷新，因为exact identity事实
没有变化。

## 3. Delivered real receipt

### Canonical source/artifact producer

Router helper通过现有`skiff-package-service-smoke-fixture`先bootstrap canonical std，再使用
`--prepare-host-base`调用S0同一个
`prepare_package_service_host_fixture`。artifact/work/receipt全部位于测试owned临时root，
environment精确保持`current-scope`，所以生成identity与S0 bit-exact。

Host测试直接调用该Rust producer、从canonical store读取base assembly，并使用
`FilesystemRuntimeAssemblyContentResolver`解析完整closure。它成功admit三个exact route：
HTTP unary、HTTP server-stream和connect-only WebSocket。没有进入旧fixture的schema-neutral
std/package rewrite。

Router compatibility测试使用`FilesystemRuntimeAssemblySnapshotLoader`读取同一closure，确认
consumer/provider deployment与contract、consumer Actor File IR和三个exact gateway binding。
unary与stream测试把loader返回的binding装入active snapshot，通过真实HTTP gateway、
runtime dispatcher和ephemeral runtime WebSocket执行：

- unary：exact routing identity/generation/selector到达runtime，返回`201`、header与opaque body，
  pending owner归零；
- server stream：exact routing到达runtime，返回`206`、header、两个ordered chunks、一个end，
  pending/stream/backpressure owners全部归零。

tracked source静态receipt仍为12个nested `timeout(...)`节点，并保留HTTP unary/stream、
三参数`requestJsonToConnection`、file、Actor和slash service call。I6 final acceptance在同一
production baseline上已覆盖deadline/tie-break/late-settlement和service-call current carrier；
S1又重跑五类carrier delivery selector，production相对I6/S0为零diff。

## 4. Evidence ledger

| 层级 | 命令 | 结果 | 覆盖 |
| --- | --- | --- | --- |
| Host exact artifact | `cargo test -p skiff-runtime-host host_current_scope_compiled_artifact --locked` | PASS `1/1`；其它targets非零selector为0且明确filtered | exact S0 producer/store/resolver/admission与三route identities |
| I6 carrier continuity | `cargo test -p skiff-runtime-eval f445h_i6_carrier_delivery_receipt --locked` | PASS `5/5` | HTTP unary、WebSocket request、file、Actor、time current carrier到lower API |
| Router focused matrix | I7R列出的5个Vitest files | PASS `55/55` | exact source/artifact reader、unary、stream、WS protocol/dispatch、late terminal与wrong identity/generation negatives |
| Rust locked check | `cargo check -p skiff-runtime-host --tests --locked` | PASS | Host test wiring；只有baseline既有warnings |
| Router type-check | `pnpm --dir router type-check` | PASS | TS helper/fixtures/current loaded binding types |
| F442 self-test | `node .../verify.mjs --self-test` | PASS；`controls=6 rawCases=79` | corpus self mutation |
| F442 combined | `node .../verify.mjs --combined-probe` | PASS；`activation-parity` | cross-system activation parity |
| runtime wire | `node .../verify.mjs --runtime-wire-self-test` | PASS；activation `6/7`，request mutations `115`等全部reported | current request/wire strictness |
| source/static | timeout与carrier `rg`；implementation diff forbidden scan | PASS；12 timeout，六类operation命中；新增`$/cancelRequest`、`-32800`、`CancelError`、`peerRequestId`、legacy relay、`ServiceTimeoutConfig`为0 | real source与non-goal absence |
| format/hygiene | `cargo fmt --all -- --check`; `git diff --check` | PASS | Rust format与全写集whitespace |

Router依赖没有安装或修改；测试临时使用integration checkout已有的source-compatible
`router/node_modules`只读symlink，验证后已unlink。所有Node临时artifact roots由`afterAll`删除。
Host current-scope fixture使用单次局部owner，在admission后Drop并删除root；收尾确认无匹配S1
Host临时目录残留。开发中发现的两个旧S1临时目录已精确删除，不可恢复且只包含本任务生成物。

## 5. RED classification and convergence

首次Host RED显示assembly identity不是S0值，原因是test-only producer使用了新的environment
`s1-current-scope`；RuntimeAssembly identity正确包含environment事实。改为S0冻结的
`current-scope`后exact identity命中并admit，不是production defect。

首次Router reader断言错误地检查`LoadedRuntimeAssembly.resolvedPackages`，而该公开Router
snapshot刻意只暴露deployment/contract/gateway与actor method投影。修正为断言Actor method后，
filesystem loader仍真实读取consumer PackageArtifact/File IR。没有改变production surface。

收尾发现静态`OnceLock`会保留新增Host temp root；改成单次局部fixture owner并重跑Host selector、
locked check、fmt与cleanup检查。没有其它blocker。

## 6. Acceptance and invalidation

| 条款 | 判定 |
| --- | --- |
| tracked S0 source → exact artifact closure | PASS |
| exact artifact → Host filesystem admission | PASS |
| exact artifact → Router filesystem load/join | PASS |
| exact unary/stream binding → observable HTTP result | PASS |
| status/header/chunk order/single terminal | PASS |
| current-scope carrier continuity and three-parameter hidden-id source | PASS |
| wrong identity/generation, legacy/public cancel and late terminal fail closed | PASS |
| production/public/wire/schema/identity zero diff | PASS |
| hermetic temp ownership and cleanup | PASS |

```text
BLOCKING_ISSUES = 0
S1_COMPLETE = YES
J_S1_PREREQUISITE_SATISFIED = YES
```

S0 fixture/source or identity、canonical producer/std、Host resolver/admission/execution、
Router artifact reader/transport、I6 production或F442 corpus任一变化会使相应证据失效。
S1没有运行full gate、stable、reload、Mongo、external network、OAuth、browser或live；
这些仍由J/L0/L1各自唯一owner负责。
