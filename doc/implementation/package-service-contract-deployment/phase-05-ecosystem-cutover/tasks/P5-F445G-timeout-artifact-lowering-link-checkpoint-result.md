# P5-F445G Timeout artifact, lowering and link checkpoint result

状态：`I3_COMPLETED / REVIEW_FINDINGS_CLOSED / FULL_COMPILER_GATE_RETAINS_7_INHERITED_TARGET_FAILURES`。

F445B-I3 已把 F445E source execution plan 完整投影到持久 File IR 和 linked program。独立
review 首轮结论为 `FAIL`，指出两个 artifact admission 缺口和一个被前置错误遮挡的 negative
test；三项均由独立 correction 闭合。File IR 的 Router current reader 也因不在 I3 原写集内，
由独立 child 精确迁到 v9。本文不把仍存在的 compiler/Router input fixture failures伪报为
full gate GREEN。

## 1. 输入、提交与组成

| 项 | Commit | Tree |
| --- | --- | --- |
| 任务指定 integration input | `b8aba75be2a0d282533268737c60092a4003c70f` | `d81ef76e487fa913f21dfb82043c7a7b870ea826` |
| I3 有效 task HEAD；相对 input 只有 task 文档 | `929bea76b7529e6f1b0ec0e7482347befcf76cb0` | `689b67ef75875615abd9ef69025a6fbf04ea6d6e` |
| I3 implementation | `dee2d0b5d67df9a6f3358d68ee835c7695680e21` | `4530396f1fc4bb3bad4aec253f206209227348a8` |
| Router v9 admission implementation / result | `a4b1926d7e21b80b4e036c241a16cebbf462dd60` / `a1ad5a3e7dddb4d2c4ad19a3a95c243e1d9ec71f` | `304375a15a73b3e04fe27e86575f32e694da877d` / `7b6c5384ebc726e38a35b38e7bb8238fa74a0f93` |
| 独立 I3 review result | `b29954880dc90a69e867f105fc4a87dd5eeeae0b` | `88b0989141f3b1c43a84a3fd55fd0eac1f5de102` |
| admission correction implementation / result | `32f60de6659ae9ef93998d6673437c5b21c9fcc5` / `9b769e80e1fab59ae61035e59a27ed2e43f87a74` | `99c26ecf35d37a89b6e3c1c9a650eee00df6b4c7` / `cbbf73aab86bad6711eb993ee31087ad367d777b` |
| 全部组成整合后的 integration | `14545e30fd521b1844e10a8c58462e7cf1f29a5c` | `5e7ed37527f17bacd71421f9c5c8bc4f35b53f38` |

integration 中对应顺序为：

```text
78aaa8bd  I3 main implementation
d5cc8ea7  Router v9 admission
9d7a2ae8  Router result
3b357efd  independent FAIL review result
21fac6b0  admission correction
14545e30  correction result
```

两处整合冲突只涉及 official std build golden，均保留由真实 v9 输出证明的
`4b64e7d3…f2a71c`；prelude、Local ABI 和 schema index 保留独立历史审计的 exact 值。

主实现使用任务指定 target：

```text
/Users/geek/workspace/skiff-p5-f445g-timeout-ir/build/cargo-target
```

主 implementation 只修改 task 允许的 artifact-model、File IR identity、compiler lowering /
direct tests、linked-program 和 linker owner。`runtime/linked-program` 的一行
`global_ingress -> gateway_ingress` 来自独立 F358-R1 current-model fixture closure；compiler
identity fixture 中与 timeout 无关的 schema index、Local ABI 和 prelude 值均有 F445C-R1–R4
双历史点证据。主 implementation 没有派子 Agent；Router、review 和 correction 是 coordinator
为写集隔离和独立验收调度的 sibling leaf。

## 2. Test-first 证据

production 修改前先加入了三个独立 owner 的测试：

- `artifact-model/src/executable/timeout_execution_tests.rs`
- `compiler/tests/timeout_artifact_lowering.rs`
- `runtime/linker/src/linker/file_conversion/timeout_execution_tests.rs`

首次 artifact focused 运行 exit `101`，明确缺少 `ConcurrentPlanIr` /
`ConcurrentLaneIr` 以及新的 statement/expression variants。同期
`cargo check -p skiff-compiler` exit `101`，精确暴露 F445E handoff 的 11 个 lowering
non-exhaustive match；随后才实现 IR 和 lowering。

Router R1 在修改前真实 RED：

- compiler-generated fixture 已产生 v9，但测试仍期望 v8；
- filesystem loader 的 v8 prefix 拒绝同一 v9 record。

独立 review 后的 correction 也保持 test-first：

1. artifact/source duration 上限锁定测试先以缺少
   `MAX_SAFE_EXECUTION_DURATION_MILLISECONDS` 的 `E0425` RED；
2. 加入常量后，linker focused 为 5 PASS / 2 FAIL，证明 unsafe duration 与 duplicate source
   id 确实仍被接受；
3. 修正后的 tail-closure test 在不改 production closure validator 时已精确命中
   `tail dependencies do not close over all prior lanes`，证明旧 `missing statement 1`
   遮挡被移除。

## 3. 持久 IR 与 lowering 合同

### 3.1 唯一 shape

| Surface | 持久 shape |
| --- | --- |
| statement timeout | `StmtIr::Timeout { duration_ms, body, site }` |
| value timeout | `ExprIr::Timeout { duration_ms, value, site }` |
| sequential value | `ExprIr::ValueBlock { block, result }` |
| statement concurrent | `StmtIr::Concurrent { plan }` |
| value concurrent | `ExprIr::ConcurrentValue { plan }` |
| compiled plan | `ConcurrentPlanIr { lanes, site }` |
| lane | strict tagged `Statement|Serial|Tail`，含 `source_order`、`dependencies`、body/tail 与 site |

artifact 和 linked model 都使用 strict tagged serde。unknown lane kind、legacy duration 字段、
多余字段、旧 File IR generation 均 fail closed。

### 3.2 Source plan 是唯一语义 owner

完整 package lowering 显式传入：

```text
PackageSourceModel::execution_semantics()
```

lowerer 按 `module_path + ExpressionOwnerKey` 选择 plan，并在 owner 结束时要求 timeout /
concurrent plan 数量精确消费。`duration_ms`、`produces_value`、lane order、kind、dependencies
和全部 source site 都直接复制 source plan。AST 只用于降低 plan 指向的实际 body/tail，并核对
statement/serial/tail shape；没有从 AST 重算 duration、dependency 或 lane kind。

standalone helper 不持有 package execution plan，遇到 execution syntax 会以
`PackageSourceModel::execution_semantics()` 缺失 fail closed。普通 `ValueBlock` 使用独立词法
scope，body 的顺序 binding 对 tail 可见；concurrent statement/serial lane 各自降低为 block，
tail 保留 expected type。

新 AST kind 已补齐 lowering、expression preorder、return scan、type inference、external ref、
publication-local ref、source hash 和 suspend analysis。timeout 本身不增加 callable 或
`maySuspend`；suspension 仍只来自被包装 body/value 和既有 call graph。

## 4. Linked conversion 与 admission

linker 在转换任何 executable 前验证当前 generation 和 execution contract，随后逐字段复制到
linked model；它不推导 source dependency、lane kind 或 tail。

最终 admission 包括：

- statement/value duration 必须位于
  `1..=MAX_SAFE_EXECUTION_DURATION_MILLISECONDS`；
- artifact 常量与 syntax 的 `MAX_SAFE_DURATION_MILLISECONDS =
  9_007_199_254_740_991` 由跨层测试锁为相等；
- site 必须是 authored source，offset 精确且正向；
- source id 必须唯一命中，且 source-map entry 的 module 必须等于当前
  `FileIrUnit.module_path`；
- block label 唯一，timeout/lane/value tail 的 block/expression ref 必须存在；
- lane order 从零连续；
- dependency 严格递增、唯一且只能指向前序 lane；
- statement plan 不得有 tail；value plan 必须恰有最终 tail；
- tail dependency 必须精确闭包全部前序 lane。

独立 review `b2995488` 的结论及闭合如下：

| Finding | Review 严重度 | Correction |
| --- | --- | --- |
| 超过 safe-integer 上限的 duration 被接受 | HIGH | statement/value 最大合法值接受；最大值加一和 `u64::MAX` 均精确拒绝 |
| execution site 可指向 foreign/ambiguous source | MEDIUM | source id 唯一解析并要求 current module；duplicate、foreign timeout、foreign plan/lane 均拒绝 |
| tail closure negative 被 stale statement ref 遮挡 | MEDIUM / TEST | 保留合法 statement table，并断言 exact closure diagnostic |

correction 没有改变 IR shape/generation、lowering、linked program、Router、eval、host 或 native。

## 5. File IR generation 与 identity

原子切代：

| Identity domain | Pre-I3 | I3 current |
| --- | --- | --- |
| File IR schema | `skiff-file-ir-v8` | `skiff-file-ir-v9` |
| File IR format | `skiff-file-ir-format-v6` | `skiff-file-ir-format-v7` |
| opcode table | `skiff-opcode-table-v1` | `skiff-opcode-table-v2` |
| identity prefix | `skiff-file-ir-v8:sha256` | `skiff-file-ir-v9:sha256` |

exact v9 goldens包括：

- service-call File IR：
  `20e92b3da085320be0c3d14b38e33fe99a32cba0f4526c1bba3a8d07004df246`
- module-split File IR：
  `838ab8236b643d4c6cb83389549e17f388f38a2a1334814a28a2728dbb11d149`
- generic lowering File IR：
  `34f6e1e65fb5854a639a1b8b9dc9d868bc18d998d95b02cd7a08b2e30d895755`
- timeout full-pipeline File IR：
  `29ef7036bb3afdc59352ed7ba211181a5a55f9323d610bb7f9a92709a4f33a87`
- official std DB File IR：
  `8f7394facf54b0b5407626e48babe8905535019c50db490f862efea59214b715`

package identity 的 pre/post 证据：

| Projection | Pre-I3 | I3 current / invariant |
| --- | --- | --- |
| official std package build | `b3a2d0e8059cbad6f90c9e9dd48376e1d7c7a9c18de6063a60c2c24b8653a112` | `4b64e7d3a7c35682fd1394274f1e18002cdedc6e60b1faecf22321de51f2a71c` |
| official std Local ABI | `4e370158a4a654c55f0e086509368ebbdf34c5bfb818d5161aca18fcb62711ac` | 不变 |
| official std schema index | `26b7640548d50a600c5e04e0b61851eb66d43b34bca65c26da99bacec2a7f577` | 不变 |
| prelude | `8ec6c2b3f4411b159d8b1b8dd2d55d036713a2533dd3aba043eb3d7fb020c76e` | 不变 |
| direct provider package build | `3b9f3647318e5da0a7698be305309f5b18f0e0cbfdf256b6fc1fd7d5162116ef` | `565fb88eb39f0933491952ab7d44d447735b7102424050b899394857da835bb8` |
| direct provider Local ABI | `2b6b70c8b858a3ee88df957eb0488a98224fd928669c84021f15aecf7de464e6` | 不变 |

build 变化来自 identity-bearing File IR generation；Local ABI、schema index 与 prelude 不随
executable body格式变化。`PACKAGE_ARTIFACT_SCHEMA_VERSION` 仍为
`skiff-package-artifact-v9`，`SERVICE_CONTRACT_SCHEMA_VERSION` 仍为
`skiff-service-contract-v5`，`RUNTIME_ASSEMBLY_SCHEMA_VERSION` 仍为
`skiff-runtime-assembly-v2`。

## 6. Router current reader closure

全仓反搜发现原 I3 写集外唯一 active current reader：

- `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts`
- `router/tests/compilerGeneratedManifestCompatibility.test.ts`

Router R1 只把这两个 consumer 从 exact v8 改为 exact v9，没有增加 v8 fallback 或 dual-read。
验证：

| 命令 | 结果 |
| --- | --- |
| compiler-generated compatibility focused | PASS：1/1 |
| dynamic-build-id parity focused | PASS：4/4 |
| Router type-check | PASS |
| 两文件 v8 反搜 | 0 match |
| Router full | 819/820 PASS；唯一失败为既有 actor-spawn error-text assertion |

`cross-system-fixtures/dynamic-build-id-parity/case.json` 的 v8/format-v1/opcode-v1 是显式跨
generation identity corpus，不是 current admission positive consumer。

## 7. 验证矩阵

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-artifact-model --no-fail-fast` | PASS：178/178；correction focused 后为 3/3 timeout tests |
| `cargo test -p skiff-artifact-identity --no-fail-fast` | PASS：134 unit + 8 CLI；1 ignored regenerator |
| `cargo test -p skiff-compiler --test timeout_artifact_lowering -- --nocapture` | PASS：4/4 |
| official std authoring focused | PASS：1/1 |
| canonical builtin identity focused | PASS：1/1 |
| provider identity suite | PASS：4/4 |
| `cargo test -p skiff-runtime-linked-program --no-fail-fast` | PASS：34 unit + 1 integration |
| `cargo test -p skiff-runtime-linker --no-fail-fast` | 主实现 55/55；review correction 后最终 58/58 |
| `cargo check -p skiff-compiler` | PASS |
| correction `cargo check -p skiff-compiler --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |
| `cargo test -p skiff-compiler --no-fail-fast` | exit `101`：7 个 inherited target、16 个 test failure；I3 target 4/4 PASS |

完整 compiler gate 的七组既有失败没有被修改或掩盖：

| Target | 失败 | 精确分类 |
| --- | ---: | --- |
| `actor_dispatch_linking` | 1 | `MissingHydratedSchemaIndex` |
| `prelude_std_schema` | 1 | stream boundary projection 未得到既有 `UnsupportedStream` 预期 |
| `root_path_references` | 1 | DB schema fixture 缺 database state requirement |
| `runtime_slots` | 7 | 既有 receiver/native projection 与 DB state fixture failures |
| `shared_fixture_lane_probes` | 3 | DB state requirement fixture failures |
| `std_package_imports` | 2 | 缺 `api.yml`；symbol count 93 vs 91 |
| `streams_emit` | 1 | 既有 native `std.http.sse` node expectation 缺失 |

这些失败均不在 timeout artifact/lowering/linker owner；任务专属真实 package lowering、
identity、strict serde 和 link validation tests 全部 GREEN。

## 8. 反向闭包与后继边界

- I3 owner 内旧 v8/v6/v1 只剩 deliberate stale-generation / stale-prefix rejection。
- active Router v8 reader 已由 R1 闭合；没有第二个 current production consumer。
- 没有修改 syntax production、`compiler/source/**`、request、capability-context、eval、host、
  native、ServiceContract 或 runtime assembly DTO。
- 没有为 timeout 新增 public callable、throw surface、legacy兼容路径或 runtime semantic
  inference。
- 没有 merge、rebase、push、stable、live 或 network 操作。

本节点交付的是 I3 持久与 link checkpoint。真正执行 linked timeout / concurrent plan 的
scope scheduler、host deadline propagation 和 consumer wiring 仍由 F445B 后继 I5/I6 节点
显式完成，不能在 runtime 从 source 或 AST 重新推导。
