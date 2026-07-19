# Phase 03 验证结果

状态：**P3-A01 初次验收 FAIL；`34b6a863` 为历史候选，等待 F04/F05 修复与新 stability epoch。**

初次 A01确认 gate ledger本身可复现，但发现两项阻塞：full-chain fixture缺少真实 consumer/service edge；runtime
artifact boundary checker漏扫真实 `request_entry.rs`。两项均为局部 evidence/structure-gate缺口，不涉及产品决策；
分别由 F04、F05修复，合流后由 T09R重建受影响证据，再交 A01复验。

Phase 03 的 canonical deployment / assembly、typed load/link 与 whole-assembly admission 已在同一候选上闭环。
本记录之后的提交只包含本文，不改变 production、fixture、Cargo、checker 或 gate 结论。

## 1. Stable candidate

- integration branch：`codex/package-service-phase-03`。
- Phase 03 基线：`629e78d67353c2efe55abb5bee942bfd20527dae`，tree
  `8eea6dd03ff6f36570e1081fc53e2c2dbf3dc493`。
- stable production candidate：`34b6a863534b435d1e81b88de4cd8c0ed8a352fa`。
- candidate tree：`bf8f13b52e8abef1d72dc3f2c07de0ed89fde449`。
- candidate 建立时工作树 clean、无 unmerged path；未 merge `main`、未 push。
- 本文不记录自身 result commit hash；result commit 由任务回报与后续 merge 记录引用。

候选之后没有 production writer。T09 的四个机械提交为：

| commit | 分类 | 结果 |
| --- | --- | --- |
| `824617f` | dependency hygiene | 删除 `runtime/package-test` 未使用的 `skiff-compiler-projection` direct dependency 与 lock direct edge；最终 DAG 证据不含该边 |
| `c0dbeb7` | missing integration evidence | 增加真实非空 `ServiceDeploymentInput -> projection -> resolution -> typed load/link -> admission -> active lookup` 测试；不增加 legacy builder |
| `3f35e32` | compile/checker mechanical seam | 删除 stale `PackageOperationSymbolRef` facade import；将两个只读 accessor 从 checker 会误识别的 `contract_operation_id` 改名为 `operation_id` |
| `34b6a86` | stale test/fixture seam | 迁移 v5/format-v3/`serviceCallRefs` fixture；旧 `packageSymbol` call 改为 strict boundary 拒绝；package-test legacy consumer 改为精确 Phase 05 fail-closed |

## 2. DAG 合流记录

下表按实现 DAG 记录 source 与 integration target。所有 merge 均无文本冲突；“owner”列记录验收暴露的
逻辑冲突或后续证据失效 owner，而不是把其伪装成 merge conflict。

| DAG 节点 | source commit | target commit | 冲突 / repair owner | 合流后的证据状态 |
| --- | --- | --- | --- | --- |
| D01 / phase plan | `149d901`, `e92f612` | direct | 无 | 冻结三波 DAG 与唯一 gate ownership |
| T01 canonical contract | `fbc3542` | `667c062` | R01 暴露 checker/public-surface checkpoint gap，退 F01 | T01 自证据失效，等待 F01 + R01 |
| F01 checkpoint repair | `37bbf6e` | `ad617de` | F01 | identity checker/self-test 与 public API characterization 重建；R01 PASS 后解锁 Wave 2 |
| T02 deployment projection | `3f72a72` | `7a549c9` | 无文本冲突 | isolated projection 证据在与 T03 合流后交 R02 |
| T03 assembly resolver | `6069fa8` | `46d7b6f` | R02 的 deployment verdict 暴露 eligibility gap，assembly verdict不回退 | combined T02/T03 证据交 F02 |
| T04 typed loader | `7abeca3` | `30372fe` | 无 | isolated loader 证据在 T06 consumer 接入后交 R03 |
| T05 shared image | `f00794c` | `8bc9800` | 无 | isolated image 证据在 T06 consumer 接入后交 R03 |
| F02 deployment eligibility | `e3c91db` | `9d57d14` | F02 / T02 boundary eligibility | deployment verdict 修复，R02 双 verdict PASS |
| T06 assembly linker | `1b132bf` | `df9b884` | 无 | T04/T05/T06 exact integration 由 R03 验收 PASS |
| T08A terminal seams | `0e0127c` | `c4847ce` | 与 T07 host test-support facade 同步时暴露 stale export | T08A isolated evidence由 F03/T07同步与T09 runtime gate替代 |
| F03 host test-support seam | `2be718f` | `59e5c51` | F03；同步节点 `3093104` | 删除 stale re-export，不恢复 alias/adapter |
| T08B boundary checker | `9abc393` | `f83a450` | 无 | isolated checker evidence在T07最终合流后由T09实扫重建 |
| T07 whole admission | `3dffef4 -> 46d33d8 -> 3093104 -> 9c94303` | `600a621` | F03只处理compile seam；request I/O probe由T07 owner补齐 | isolated host evidence在T09真实链与runtime selector上重建 |
| T09 integration | `824617f -> c0dbeb7 -> 3f35e32 -> 34b6a86` | direct | 仅机械 dependency、compile/checker、test fixture owner | 建立本文件记录的最终 stability epoch |

高风险独立 verdict 的结果：R01 在 F01 后 PASS；R02 的 assembly verdict保持 PASS、deployment verdict在 F02 后
PASS；R03 在 `df9b884` 的 typed loader/image/linker checkpoint上 PASS。T09 没有发现需退回 T01–T07 的
schema、identity、projection、resolution、link 或 admission 语义缺口。

## 3. 需求 → production code → test 证据

| 完成态要求 | production owner / 代码 | 动态与结构证据 |
| --- | --- | --- |
| 四对象 typed schema、strict wire、identity分离 | `artifact-model/src/{deployment,runtime_assembly}.rs`；`artifact-identity/src/{deployment,runtime_assembly}/**` | `deployment/src/tests.rs` strict wire、identity mutation/normalization、empty assembly与tamper；foundation与identity checker PASS |
| source-free deployment projection，不从 AST/File IR 反推 contract | `deployment/src/projection/**` | `projection_maps_every_operation_explicitly_and_emits_no_public_path`、operation/descriptor/eligibility/config/resource/capability负例；runtime DAG与boundary checker证明无 compiler dependency |
| service cycle、package diamond、唯一 provider与exact package closure | `deployment/src/assembly/{resolver,candidates}.rs` | `service_cycle_closes_iteratively_with_activation_scoped_slot_zero`、`package_diamond_links_each_build_once_and_is_input_order_independent`、零/多 provider、protocol/version/ABI/build/ingress/tamper负例 |
| typed exact-ref load与 immutable contract store | `runtime/loader/src/runtime_assembly/**` | empty no-read、contract store/code lookup、artifact/ref/File IR/resource/link-target tamper tests |
| package code每build共享，activation binding不共享 mutable owner | `runtime/linked-program/src/shared_image.rs` | same alias按caller build隔离、one code owner、caller-relative service slot、duplicate build/local ABI/protocol/missing callable负例 |
| canonical direct call与whole candidate link | `runtime/linker/src/assembly/**` | shared code + activation-relative binding、canonical descriptor/typed ingress、missing callable、ABI/protocol/ingress/template tamper；canonical call不得fallback到service-specific converter |
| whole-assembly atomic admission与失败保留 active | `runtime/host/src/loader/assembly_admission.rs` | empty admission、load/link/admit各阶段失败、in-flight generation、concurrent candidate与failed reload preserves active |
| request path不 lazy-load artifact；terminal consumer不恢复legacy | `runtime/host/src/loader/{load,runtime_config}.rs`、`runtime/{activation,eval,linked-type-plan,package-test,request}/src/*assembly_seam.rs` | T07 request-entry probe、runtime artifact boundary checker 14个mutation self-test与production零DENY；package-test builder在legacy artifact load前fail closed |
| 真实非空 producer到active lookup，不复制descriptor、不追加I/O | `deployment::projection` + `deployment::assembly` + loader/linker/admission public handoff | `runtime/host/src/loader/assembly_admission/tests/full_chain.rs::projected_nonempty_assembly_admits_and_active_lookup_is_io_free`：contract `Arc` exact identity、descriptor同引用、template无`stableKey`/`valuePlan`、lookup零额外resolver read、tampered reload保留active |

必验矩阵不是只由最后一条 happy path承担：A↔B cycle、package diamond、shared build/per-activation template、不同
caller slot 0、empty assembly、零/多 provider、Unavailable/missing operation、ABI/protocol/binding/template/ingress/
artifact tamper分别由 deployment、loader、shared-image、linker、admission 分层负例覆盖。

## 4. Gate ledger

`C` 表示 §1 的 stable production candidate。foundation/compiler在 `c0dbeb7` 上 PASS；其后的变更只触及
runtime production accessor/import或runtime tests，不触及 foundation/compiler crate、fixture、Cargo edge或
checker，因此两项证据保留。runtime DAG 在 `c0dbeb7` 上 PASS；之后没有 Cargo 变化。所有受影响的 runtime、
identity、boundary、public API、format 证据均在 `C` 重建。

| 层级 | 命令 | commit / 状态 | exit / 耗时 | 结果 |
| --- | --- | --- | --- | --- |
| foundation | `node scripts/verify.mjs --only foundation` | `c0dbeb7` / retained | `0` / 9.48s | PASS |
| compiler | `node scripts/verify.mjs --only compiler` | `c0dbeb7` / retained | `0` / 20.44s | PASS |
| runtime initial | `node scripts/verify.mjs --only runtime` | `c0dbeb7` | `1` / 18.71s | stale runtime facade import；见 §5 |
| runtime convergence | 同上 | `3f35e32` | `1` / 15.90s | production编译已闭环；旧 artifact/host/package-test fixtures 与 T08A terminal seam预期漂移；见 §5 |
| runtime final | 同上 | `C` | `0` / 10.30s | PASS；18 runtime packages、lib与doc tests；同时执行boundary self-test 14/14和production checker |
| identity initial | `node scripts/check-artifact-identity-single-source.mjs` | `c0dbeb7` | `1` / 0.44s | 两个只读accessor被constructor regexp误识别 |
| identity final | 同上 | `C` | `0` / 0.47s | PASS |
| runtime DAG | `node scripts/check-runtime-crate-dag.mjs` | `c0dbeb7` / retained | `0` / 0.09s | PASS；后续无Cargo变化 |
| runtime boundary | `node scripts/check-runtime-artifact-boundaries.mjs` | `C` | `0` / 0.88s | PASS；selector内self-test 14/14亦PASS |
| public API | `node scripts/check-crate-public-api.mjs --all-configured` | `C` | `0` / 1.49s | deployment、compiler-contract、compiler全PASS |
| targeted rustfmt | `git diff --name-only 629e78d...HEAD -- "*.rs" \| xargs rustfmt --edition 2021 --check` | `C` | `0` / 0.58s | 覆盖全部76个Phase Rust改动文件（含T09新增/修改Rust文件） |
| whitespace | `git diff --check` | `C` | `0` / 0.01s | clean candidate PASS |
| phase whitespace | `git diff 629e78d...HEAD --check` | `C` | `0` / 0.03s | 完整Phase diff PASS |

最终有效 gate 的顺序耗时合计约 43.74s；其中最后六个轻量检查实际并行墙钟约1.5s。失败收敛 probe另有：

- `cargo test -p skiff-runtime-host --lib`：330/330 PASS；
- `cargo test -p runtime --lib`：276/276 PASS；两者组合命令 wall约4.3s；
- full-chain exact test：1/1 PASS，2.59s；
- deployment 32/32、typed loader 5/5、shared image 8/8、assembly linker 7/7、admission 5/5 与
  boundary self-test 14/14 的pre-freeze probes均PASS。

## 5. 失败分类、repair owner与旧覆盖迁移

| 失败 | 分类 / owner | 修复与 replacement |
| --- | --- | --- |
| `runtime/package-test` direct依赖已不使用projection crate | mechanical dependency seam / T09 | 删除依赖与lock edge；locked metadata、depth-1 tree与runtime DAG PASS |
| 分层测试没有一条真实非空 producer→admission→active lookup链 | missing integration evidence / T09 | 新增 `full_chain.rs`，使用真实typed producer，不使用fake empty contract或legacy aggregate |
| `PackageOperationSymbolRef` stale import | mechanical compile seam / F03 surface、T09 integration | 只删除import/re-export引用，不新增alias |
| identity checker把 `contract_operation_id()` accessor当成第二constructor | mechanical naming/checker seam / T09 | 改为 `operation_id()`；字段、identity preimage与wire不变 |
| runtime/host fixture缺`serviceCallRefs`，仍写v3/format-v1 | stale test fixture / runtime driver + host test owners | 集中fixture helper升级到v5/format-v3并显式补canonical空表；production serde仍strict、没有default/dual-read |
| legacy `packageSymbol` call仍出现在service-specific conversion测试 | stale test semantic shape / runtime driver test owner | broad rewrite test保留package type、local/external rewrite，移除仅旧call子例；新增 `legacy_package_operation_call_is_rejected_at_artifact_boundary` |
| 旧“package operation不得fallback”测试依赖已删除variant | stale test / T05/T06 replacement owner | replacement为 `shared_image::missing_callable_is_rejected_while_validating_linked_call_sites`、`assembly::missing_provider_callable_is_rejected_before_linking_a_candidate`、`assembly::canonical_calls_cannot_fall_back_to_the_service_specific_converter` |
| host/router package-test测试仍期待legacy materialization成功 | stale downstream expectation / T08A terminal seam，Phase 05 consumer owner | 保留binary transport、requestId、空payload、排队/非阻塞/取消/饱和与cache不污染断言；成功预期改为精确`InvalidArtifact` Phase 05 migration boundary |
| shared dynamic-build cross-system fixture仍是v3 | downstream golden migration / Phase 05 + identity owner | 不改共享golden、不写runtime-local假golden；runtime精确断言strict boundary拒绝。当前v5路径由真实full-chain与loader tests覆盖 |

以上均为机械 integration/test evidence seam。T09 没有放宽 strict schema、恢复 legacy variant、增加 adapter/
fallback/dual-read，也没有自行修改 projection/resolution/link/admission 产品语义。

## 6. Phase 04 / Phase 05 未运行项与残余风险

- **Phase 04 gate 未运行。** 本阶段只证明 activation/service binding template 与 canonical descriptor lookup；没有
  实例化或传播 ActivationContext，也没有验证dispatcher、same-heap materialization、async/stream/callback/
  cancellation执行语义。
- **Phase 05 gate 未运行。** registry/pointer/storage authoring、router control/reload、成功的package-test legacy
  service-program materialization与旧service consumer迁移均不属于本阶段。相关host入口当前精确fail closed。
- 未运行router、test-runner、telemetry、live、chat smoke或跨仓库gate；这些不能替代Phase 03 typed control-plane
  证据。
- `cross-system-fixtures/dynamic-build-id-parity/case.json`仍是v3共享golden；必须由Phase 05/identity owner跨
  compiler/runtime/router原子迁移。当前runtime拒绝它是预期的strict boundary，不应通过局部重写expected hash绕过。
- runtime gate保留少量dead-code warning，包括等待Phase 05 consumer的`package_test_service_context`以及既有host
  control/cache辅助函数；没有warning升级为schema或call-graph blocker。
- stable evidence只对 `34b6a863...` / tree `bf8f13b5...` 有效。修改 production owner、Cargo edge、checker、
  fixture、public surface或gate环境时，必须按影响面重建证据；不能拿task branch旧PASS替代。

## 7. 下一验收边界

P3-A01应在本文result commit的clean tree上独立只读验收，并确认排除本文后production tree仍等于
`bf8f13b52e8abef1d72dc3f2c07de0ed89fde449`。A01不得把Phase 04/05未实现的consumer执行能力计为本阶段
blocking，也不得把上述v3 shared golden降级成runtime兼容路径；若发现四对象owner、identity、projection、
resolution、link或admission语义缺口，则按原owner返回，而不是由result记录补语义。
