# Phase 04 验证结果

状态：**P4-T10 COMPLETE；Phase-scoped requirements/gates无blocker，frozen production candidate为
`13b4600f` / tree `a34e103c`，可提交P4-A01独立验收。Repo `router`与`checks` selector仍有明确的
inherited baseline failures，本文不把这两个命令写成PASS。**

Phase 04的assembly execution image、ActivationContext、ordinary/error、async/stream/cancel、
callback/native、unified ingress与router service-relay retirement已经在同一production candidate上闭环。
F16只迁移eval error wrapper owner；其后唯一受影响的完整runtime selector与聚焦checker已重建。Stable
candidate binary provenance最终恢复为四方一致；Agine smoke在Phase 05尚未提供active assembly ingress时精确
fail closed，分类为`EXPECTED_PHASE05`，不是Phase 04 PASS证据。

本文是T10 result draft。P4-A01尚未给出独立verdict，因此本文不宣布Phase 04 COMPLETE，也不授权merge
`main`或push。

## 1. Frozen candidate与stability epoch

- integration branch：`codex/package-service-phase-04`。
- Phase 04代码基线：`0cbf533409392066d7960e1c2afbdb129defaa9f`，tree
  `5893175665e1f47415b0537e25510446613bf08f`。
- 历史T10 candidate：`f093e921a6c7961c5d727deeb83a2b6fd78adb94`，tree
  `c169eb8b100ea93fd4e7d1b1a6003777d67dc51d`。该候选的runtime selector通过，但eval error-boundary
  checker发现4个candidate DENY，故不再作为最终候选。
- 最终production candidate：`13b4600f38ae1d0cdc6878ecb518e2b616d5e4fa`。
- 最终candidate tree：`a34e103cb8a95f0611b380ae3a173266471fcc6d`。
- F16 source为`59dfb219290346b623c95d35f25e0a121e8ad9da`，integration merge为`13b4600f`。
  `f093e921..13b4600f`的production diff只有`runtime/eval/src/error.rs`、
  `runtime/eval/src/assembly_execution/boundary_materialization.rs`与
  `runtime/eval/src/assembly_execution/boundary_materialization/tests.rs`。
- 最终runtime重验、stable build/install/recovery与chat smoke前后，candidate HEAD/tree均exact且工作树clean。
- 本文与T10/phase-plan状态更新属于单独doc-only result commit；它不改变production、Cargo、checker、
  fixture或gate结论。本文不记录自身commit hash，由任务回报与后续A01记录引用。

## 2. DAG、owner与commit矩阵

下表记录source commit到integration target。`Rxx`为只读验收，不产生production source commit；其记录提交只更新
阶段文档。所有短hash均可由本节的最终candidate历史解析为完整hash。

| DAG节点 | source commit | integration / verdict commit | owner与最终状态 |
| --- | --- | --- | --- |
| D01 / phase plan | `bcd67d9`、`26ccb76`、`0ed310c`、`356f397` | direct | 冻结三波DAG、write ownership与唯一gate owner |
| T01 assembly execution image | `a6e154c` | `7efa143` | linked-program/linker immutable code owner；后续F04补共享call validator |
| T02 activation boundary kernel | `fa6d64b` | `b654634` | activation/boundary/capability owner；后续F03补cleanup/rollback |
| F01 package-test exhaustiveness | `0f3c44c` | `3f6cded` | 机械API fallout收敛 |
| T03 kernel eval handoff | `fab0c3a` | `ef14a08` | eval中央projection与lane seam owner |
| F02 assembly execution projection | `d2d81af` | `9eaea40` | assembly-backed interpreter/type/const/nested-call projection闭环 |
| F03 capability cleanup/rollback | `bf7a6dd` | `5bf3903` | payload drain、tombstone与projection rollback闭环 |
| F04 linker call validation | `f7b72e8` | `c2c27c7` | legacy/assembly traversal共享semantic validator |
| F05 callback projection ABI | `0832d2c` | `c0c2244` | F03 RAII projection ABI与eval shell机械接线 |
| R01 kernel acceptance | read-only | candidate `c0c2244`；记录`90b4a87` | 最终PASS，解锁Wave 2 |
| T04 ordinary/error | `605c467` | `6c3a045` | ordinary/error与package-direct contrast owner |
| T05 async/stream/cancel | `8e7c703` | `d0ae49f` | async/stream/cancel owner |
| T06 callback/native | `28c4689` | `ee1609c` | callback/native capability owner |
| F06 shared materialization | `1e737dc` | `7e6b039` | lane-neutral parameter/return/typed-error planner |
| F07 callback projection | `81f9bc0` | `e685519` | exact contract/local interface operation mapping |
| F08 async/stream integration | `684ffa8` | `9809dee` | async typed error、callback stream item与host合流 |
| F09 stream terminal/drop | `24fce7c` | `484cab0` | terminal publication、runtime/producer环与异常consumer cleanup |
| F10 pull/file cleanup | `72d3bca`、`c574399` | `ae7b601` | pull terminal与native file共享cleanup owner |
| R02 lanes acceptance | read-only | candidate `ae7b601`；记录`5af5626` | 三个lane及lifecycle最终PASS，解锁Wave 3 |
| T07 unified ingress | `54aa166` | `dff8414` | host/request canonical ingress与active generation owner |
| T08 router relay retirement | `7426ffe` | `ed51841` | router runtime-originated service relay拒绝owner |
| T09 checker engine | `23e3fe6` | `a65a79d` | hermetic execution-boundary checker/self-test |
| T09R production registration | `73c56c5` | `5ba7273` | merged production subject与verify接线 |
| F11 string camouflage | `a073df0` | `b0d144d` | Rust/TypeScript lexical false-negative收敛 |
| R03 entry/remote acceptance | read-only | candidate `b0d144d`；记录`172d6e8` | 三个verdict最终PASS |
| T10 initial stability epoch | integration mechanical `453c11f` | candidate `453c11f` | runtime gate暴露fixture、full-chain、stream与gate mechanics缺口 |
| F12 canonical host fixture | `b7e92d8` | `a053d7b` | 旧host request tests迁移；production零修改 |
| F13 typed full-chain fixture | `a0c5760` | `71e7ab0` | provider有效body与最终结果断言；production零修改 |
| F14 / F14R stream supervision | scope report后由`b66f92d`完成 | `f093e921` | same-handle registry + supervised consumer/drain exact-once owner |
| F15 final gate mechanics | `4948056` | `da7ce64` | Phase rustfmt、command policy ledger与依赖环境准备 |
| T10 historical retry | read-only | `f093e921` | runtime/type-check/Phase checkers证据；eval error checker暴露F16 |
| F16 eval error owner | `59dfb21` | `13b4600` | wrapper递归下沉到`runtime/eval/src/error.rs`唯一owner |
| T10 final retry | read-only | `13b4600` / tree `a34e103` | 受F16失效的完整runtime selector重建；冻结A01输入 |

## 3. 完成标准 → 真实入口/正负证据/owner/commit

| 完成标准 | production真实入口 | 正向证据 | 关键负例 | owner / commit |
| --- | --- | --- | --- | --- |
| assembly-wide immutable execution image；每build只链接一次；package direct与service instruction分离 | `runtime/linked-program/src/{assembly_execution,shared_image}.rs`；`runtime/linker/src/assembly_execution/**` | `assembly_execution_package_diamond_has_one_dependency_code_owner`、`candidate_keeps_code_shared_and_service_bindings_activation_relative` | missing callable、wrong local ABI、protocol、file ref/index与identity-valid native/interface/receiver tamper在image/admission前拒绝 | T01/F04，`7efa143`、`c2c27c7` |
| activation/generation mutable owner隔离；caller build+slot绑定明确 | `runtime/activation/src/{context,request_context,capability}.rs`；`runtime/host/src/loader/active_assembly_context.rs` | `activation_context_isolates_same_package_build_across_deployments`、explicit switch/receiver restore与active-generation pin tests | wrong activation/runtime/generation、missing operation/binding、failed reload均fail closed且保留active | T02/F03/T07，`b654634`、`5bf3903`、`dff8414` |
| physical service edge只走InProcessBoundary并做detached materialization | `runtime/eval/src/assembly_execution/{ordinary,boundary_materialization}.rs`；`runtime/boundary/src/service_linkable*.rs` | `typed_execution_ordinary`、`ordinary_in_process_uses_shared_planner_for_detached_parameters_and_return` | missing provider/no router、schema/value-plan/shape mismatch、undeclared typed throw与wrong target在invoker前失败 | T04/F06/F16，`6c3a045`、`7e6b039`、`13b4600` |
| future/continuation/stream/cancel显式传播owner并exact-once清理 | `runtime/eval/src/assembly_execution/async_stream_cancel.rs`、`runtime/eval/src/program_stream.rs`、`runtime/host/src/capability_context/stream_runtime/**` | `typed_execution_async_stream_cancel_reaches_owned_provider_future_full_chain`、provider suspend/receiver restore、六个in-process stream与stream runtime矩阵 | pre-cancel、pending next、early break、owner drop、buffered End/Error、pull/file error与consumer-first error均无task/registry/lease泄漏 | T05/F08–F10/F14R，`d0ae49f`、`9809dee`、`ae7b601`、`f093e921` |
| callback/native只投影opaque request-scope capability | `runtime/eval/src/assembly_execution/callback_native.rs`；`runtime/activation/src/capability.rs`；`runtime/boundary/src/{service_linkable,persistent,recoverable}.rs` | typed callback/native full-chain与`in_process_callback_resolves_only_declared_callback_contract_operations` | wrong tuple/mapping/runtime/activation/generation、request/stream/cancel/owner终止稳定expired/unavailable；DB/recoverable在hook前拒绝 | T06/F03/F07/F08，`ee1609c`、`5bf3903`、`e685519`、`9809dee` |
| package direct保持same heap，service boundary保持detached | `runtime/eval/src/assembly_execution/{mod,ordinary}.rs` | `package_direct_same_heap_uses_canonical_executor_and_exposes_callee_mutation`与同fixture ordinary detached对照 | service参数/返回使用fresh heap；package direct missing executable不借service materializer/fallback | T04/F06，`6c3a045`、`7e6b039` |
| typed artifacts真实到达provider execution | deployment projection/resolver → runtime loader/linker/admission → `dispatch_in_process_boundary` | `typed_execution_fixture_uses_projected_admitted_targets`、ordinary、async owned-provider、callback/native与ingress/internal filters都得到最终业务结果 | empty/missing executable body仍由专属validation负例拒绝；fixture不手写resolved target、不恢复checkpoint error | T03/F13，`ef14a08`、`71e7ab0` |
| ingress/internal命中同一dispatcher并pin active generation | `runtime/host/src/host/request_entry.rs`、`runtime/eval/src/assembly_execution/ingress.rs`、assembly admission active set | `in_process_request_entry`、`active_generation_context`与failed reload preserves active | missing/ambiguous selector、legacy callable adapter、build/operation/display mutation不触发route registry或artifact I/O fallback | T07/F12/F13，`dff8414`、`a053d7b`、`71e7ab0` |
| router拒绝runtime-originated service relay，保留其它控制语义 | `router/src/router/runtimeEndpoint.ts`的`request.start` service caller拒绝分支 | router聚焦测试证明unary/serverStream均在routing/pending前拒绝；gateway、actor/spawn回归由router suite覆盖 | service caller不能lazy-load、选runtime或建立forward生命周期 | T08，`ed51841` |
| production旧边/TLS/shared callback/第二dispatcher不可伪装或漏扫 | `scripts/check-runtime-execution-boundaries.mjs`及production subject registry；artifact boundary checker | R03 execution self-test 30/30、artifact self-test 16/16；最终production checker PASS | rename/move/duplicate/omission/test-only、Rust raw string、TS template/interpolation、TLS、remote relay等mutation均命中稳定ID | T09/T09R/F11，`a65a79d`、`5ba7273`、`b0d144d` |

上述真实链由runtime selector中的18个runtime packages共同覆盖，不由单个happy-path测试代替所有完成态。
Package-direct same-heap与service detached使用同一typed fixture做对照；async/stream/callback负例继续走production
projection、admission与dispatcher，而不是test-only resolved target。

## 4. 证据保留与失效边界

1. `f093e921`冻结前执行`node scripts/verify.mjs --list`一次，得到194个去重phase。Runtime selector已经展开
   artifact-boundary self-test、production checker与18-package Rust gate；`checks`展开compiler/public API/
   identity/package-store及后续runtime/checkers。因此T10没有把展开项冒充多次独立执行。
2. `f093e921..13b4600f`只有三份runtime/eval Rust文件和两份Phase文档变化。F16使runtime selector、
   eval error-boundary、三文件rustfmt与F16 diff证据失效；这些均在最终tree重建。
3. F16没有修改router、TypeScript/JavaScript、Cargo/lock、其它checker、host fixture或stable config。因此
   `f093e921`的type-check、router baseline分类、runtime DAG、runtime execution/artifact checker、
   public API/identity/source-layout/loop-risk等证据按exact path diff保留。
4. `compiler/Cargo.toml`及router失败的compiler-generated fixture/helper相对main没有Phase 04改动；
   manifest只有lib/tests、没有bin，而既有fixture仍执行`cargo run --manifest-path compiler/Cargo.toml`。
   Router 5项失败与checks package-store失败因此分类为main同源baseline，而不是将失败命令改写为PASS。
5. stable binary由final candidate isolated target构建；最终candidate file、installed file、PID-recorded identity与
   status current identity的hash/size四方一致。任何production、Cargo、checker、fixture或gate环境变化都会结束
   当前epoch。

## 5. 分层gate ledger

### 5.1 环境与去重

| 层级 | 命令 | owner | commit / 状态 | exit / 耗时 | 结果 |
| --- | --- | --- | --- | --- | --- |
| dependency materialization | router、telemetry、vscode各自`pnpm install --frozen-lockfile` | T10 environment | `f093e921` clean；pnpm 10.29.2 | 三者`0` | PASS；三份lockfile hash前后不变，node_modules ignored |
| phase expansion | `node scripts/verify.mjs --list` | T10唯一owner | `f093e921` | `0`；未单独计时 | PASS；194个去重phase，只执行一次 |
| Phase Rust闭集 | `git diff --name-only 0cbf533...f093e921 -- '*.rs' \| sort -u \| xargs rustfmt --edition 2021 --check` | T10 | `f093e921` | `0` / real 0.90s | PASS，129个Phase Rust文件 |
| repo-wide fmt审计 | `cargo fmt --all -- --check` | F15 | `da7ce64` | 非零；未提供数值/耗时 | **不是PASS**；只剩3个Phase外main baseline：`runtime/host/src/host/http_runtime/tests/{egress,helpers,stream}.rs`。main还会额外命中已由F15收敛的Phase文件`runtime/capability-context/src/http.rs` |

Frozen install前后lockfile SHA-256保持：
router `dab9831e0d24f9ab18d9940662356350f43dc12d8f95c19155ccac28a5262cf4`、
telemetry `7ded5a40a51ee4ac7c53afba21790734ee301e9dfbec9736b2dbc52f6f5d264f`、
vscode `3491e27a8c21128697e1e60cb9b9b601446cd9e34a6e5b3adc7a946a1e88b284`。

### 5.2 `f093e921`首次T10 epoch

| 层级 | 命令 | owner | commit / 状态 | exit / 耗时 | 结果 |
| --- | --- | --- | --- | --- | --- |
| runtime | `node scripts/verify.mjs --only runtime` | T10 runtime shard | `f093e921` | `0` / real 21.89s | PASS，3/3：artifact self-test 16/16、production artifact checker、18 runtime packages unit/doc tests |
| runtime动态重点 | 上述runtime selector内 | T10 runtime shard | `f093e921` | 同上 | typed full-chain、ordinary/error、package-direct same-heap、9项async/stream/cancel、callback/native、unified ingress/generation、F14R lifecycle均PASS |
| router | `node scripts/verify.mjs --only router` | T10 router shard | `f093e921` | `1` / wall 4.714738s | **FAIL**；24 files中21 pass/3 fail，384 tests中379 pass/5 fail；均由compiler `cargo run`无bin触发，main同源baseline |
| type-check | `node scripts/verify.mjs --only type-check` | T10 router/type shard | `f093e921` | `0` / wall 7.513992s | PASS；router、telemetry、vscode TypeScript与全JS syntax |
| checks selector | `node scripts/verify.mjs --only checks` | T10 checks shard | `f093e921` | `1` / real 16.29s | **FAIL**于`checks:package-store-discovery`；`skiff test explicit/pkg --packages-dir explicit-store`下游compiler `cargo run`无bin，main同源baseline |
| checks到首错前 | 上述checks selector内 | T10 checks shard | `f093e921` | 同上 | compiler boundaries、command policy、compiler DAG self/production、public API self/all、local instance、artifact identity self/production均PASS |
| publication archive | `node scripts/check-publication-resource-archive.mjs` | T10 checks shard | `f093e921` | `0` / 0.06s | PASS；因checks fail-fast未到达而补一次 |
| runtime DAG self | `node scripts/check-runtime-crate-dag.mjs --self-test` | T10 checks shard | `f093e921` | `0` / 0.05s | PASS，10/10 |
| runtime DAG production | `node scripts/check-runtime-crate-dag.mjs` | T10 checks shard | `f093e921` | `0` / 0.09s | PASS，17 promoted crates |
| runtime execution boundary | `node scripts/check-runtime-execution-boundaries.mjs` | T10 checks shard | `f093e921` | `0` / 1.27s | PASS，production零违规 |
| eval error boundary | `node scripts/check-runtime-eval-error-boundary.mjs` | T10 checks shard | `f093e921` | `1` / 0.22s | **FAIL，candidate blocker**；`boundary_materialization.rs:309/313/318`共4个DENY，退F16 |
| source layout | `node scripts/check-skiff-source-layout.mjs` | T10 checks shard | `f093e921` | `0` / 0.06s | PASS |
| loop-risk self | `node scripts/check-loop-risk-health.mjs --self-test` | T10 checks shard | `f093e921` | `0` / 0.05s | PASS |
| whitespace | `git diff --check` | 两个T10 shard | `f093e921` | 两次均`0` | PASS，但违反“同一候选唯一owner只跑一次”；ledger如实记录重复两次，不伪称唯一执行 |

`checks`在package-store处fail-fast，所以后续项不是由该selector执行；表中七个补充项各执行一次。Artifact
boundary checker已经由runtime selector展开，未另行重复。Eval error boundary的4个DENY均由Phase commit
`1e737dc`引入，是当时唯一candidate-native checker blocker。

### 5.3 F16与final `13b4600f` epoch

| 层级 | 命令 | owner | commit / 状态 | exit / 耗时 | 结果 |
| --- | --- | --- | --- | --- | --- |
| F16 rustfmt | targeted rustfmt，3个F16 owned Rust files | F16 | source `59dfb21` | 整体成功；未单列exit/耗时 | PASS |
| typed error | `cargo test -p skiff-runtime-eval service_error_boundary` | F16 | source `59dfb21` | 成功；未单独采时 | PASS，1/1，52 filtered |
| wrapper recursion | `cargo test -p skiff-runtime-eval replace_user_exception` | F16 | source `59dfb21` | 成功；未单独采时 | PASS，2/2，51 filtered |
| eval error boundary | `node scripts/check-runtime-eval-error-boundary.mjs` | F16 | source `59dfb21` / final tree同一production内容 | 成功；未单独采时 | PASS，51 production files，4 DENY → 0 |
| F16 whitespace | `git diff --check` | F16 | source `59dfb21` | `0`；未单独采时 | PASS；这是新epoch的F16 focused evidence |
| final runtime | `node scripts/verify.mjs --only runtime` | T10唯一runtime owner | `13b4600f` / tree `a34e103` exact clean | `0` / real 11.39s，user 12.63s，sys 6.28s | PASS，3/3；artifact self-test 16/16、production checker、18 runtime packages unit/doc tests零失败 |

因此最终Phase-scoped runtime、type-check与Phase checker证据无blocker。Repo-wide selector结论仍必须写成：

- `runtime`：PASS；
- `type-check`：PASS；
- Phase-specific artifact/runtime-DAG/execution/eval-error/source-layout/loop-risk等checker：PASS；
- `router`：FAIL，inherited compiler-no-bin baseline；
- `checks`：FAIL，inherited package-store/compiler-no-bin baseline。

## 6. Stable runtime provenance、恢复与chat smoke

### 6.1 恢复ledger

| 顺序 | 命令 / 操作 | exit / 耗时 | 结果与分类 |
| --- | --- | --- | --- |
| 1 | candidate执行`node scripts/build-dev-runtime.mjs --config /Users/geek/workspace/skiff/.skiff-instance/config.yml --dev-home /Users/geek/workspace/skiff/.skiff-instance/dev-home` | `1` / 10.734799417s | candidate runtime和identity分别编译成功并原子安装；仅auto refresh因`runtime is unhealthy` guard拒绝。candidate/installed一度同hash `b8be0840…ab3c4`、94,136,520 bytes |
| 2 | stable worktree执行`node scripts/skiff.mjs instance restart .skiff-instance/config.yml runtime` | `0` / 22.909695583s | **过程偏差**：restart固定从脚本source root build，编译并覆盖为main binary `46cf066e…b3fa6`、91,437,928 bytes；旧pid16161停止，错误main pid28587启动，registration仍为0，不运行chat |
| 3 | candidate worktree用absolute canonical config执行`instance restart` | `1` / 0.732766833s；cargo 101 | build阶段4个E0432后停止，未install/stop/start。分类为stable main刚写入共享`~/.cache/skiff/cargo-target`后，同名同版本dependency artifact跨worktree污染；candidate isolated target此前已完整编译且源码exports存在 |
| 4 | temp config只读预检 | 未执行restart | 临时config仅改`cargoTargetDir`，但`configPath`参与instance identity；`instance status`显示全部orphaned。为避免PID metadata/config identity漂移而停止，临时文件由root删除 |
| 5 | candidate执行`node scripts/build-dev-runtime.mjs --no-refresh --config ... --dev-home ...` | `0` / 0.573613666s | 使用candidate isolated target重新安装candidate binary，明确跳过refresh；canonical status正确显示pid28587相对新installed binary为stale |
| 6 | stable worktree执行`node scripts/skiff.mjs instance refresh-binaries .skiff-instance/config.yml` | `0` / 0.872987959s | supported no-build handoff：停止pid28587、清旧metadata、启动candidate pid31241；canonical instance/config identity恢复 |

最终canonical stable runtime：

- PID `31241`，`category=running`、`binary=current`、`metadataMatchesSpec=true`；
- instance id `04b8d3c7588f6bc2dca5473a`，canonical config path
  `/Users/geek/workspace/skiff/.skiff-instance/config.yml`；
- candidate isolated file / installed stable file / PID-recorded identity / status current identity四方一致：
  sha256 `b8be0840ece3bc7abf96eb613aaaa8cfcac7c958b3a3c3959ddddb025c7ab3c4`，
  size `94,136,520`，`matches=true`；
- 4001 health `ok=true`，manifest与3个service builds存在，但`runtimes=[]`持续超过4分钟，即registrations=0；
- candidate本次启动只有keyring loaded、router connected、artifact loader configured；`runtime.err`中的旧
  stack-overflow记录mtime仍为2026-07-13，本次candidate启动没有新增panic/stack overflow，进程保持存活。

### 6.2 唯一Agine smoke

| 命令 | candidate / runtime | exit / 耗时 | 精确结果 |
| --- | --- | --- | --- |
| `/Users/geek/workspace/internals/agine`下`npm run e2e:chat-smoke` | `13b4600f`；pid31241；四方hash一致 | `1` / 0.215242292s | `session/create`在`/session`精确失败：`no active assembly ingress matches IngressSelector { protocol: Http, host: "127.0.0.1:4000", method: Some("POST"), path: "/session" }` |

分类为`EXPECTED_PHASE05`：registrations=0且Agine consumer尚未迁移到Phase 05 registry/authoring/release路径。
这不是Phase 04 PASS证据，也不是candidate panic/crash/transport blocker。Smoke只运行一次，未修复、未重试；
之后pid/status/hash与candidate exact-clean状态不变。

## 7. 历史失败分类与replacement

| ID / 候选 | 历史失败 | 分类 / owner | replacement与最终状态 |
| --- | --- | --- | --- |
| F01 | T01新增两种`LinkedCallTarget`后package-test `scan_call` non-exhaustive | mechanical API fallout / T01 consumer | package-direct exact edge与activation-relative fail-closed补齐；`3f6cded`收敛 |
| R01@`ef14a08` / F02 | canonical target解析后interpreter/EvalContext/nested call仍只消费legacy projection | candidate execution-seam blocker / T03 | assembly-backed projection闭环；F02 `9eaea40`后RESOLVED |
| R01@`ef14a08` / F03 | expired capability仍持有payload，destination allocation失败无rollback | candidate lifetime/transaction blocker / T02 | active/tombstone、drain与RAII rollback闭环；`5bf3903` |
| R01@`ef14a08` / F04 | assembly traversal未复用native/interface/receiver semantic validation | candidate validation-owner blocker / T01 | 两条link traversal委托唯一validator；`c2c27c7` |
| R01@`9eaea40` / F05 | F03改变RAII projection返回ABI，eval shell仍实现旧签名而E0053 | mechanical integration ABI / T03 | 仅同步callback shell与projection clone seam；`c0c2244`，R01最终PASS |
| R02@`ee1609c` / F06 | sync/async materializer重复，async漏typed error；package-direct对照证据不真实 | candidate cross-lane abstraction + evidence gap / T04 | lane-neutral planner与canonical package executor；`7e6b039` |
| R02@`ee1609c` / F07 | contract ID被当local ABI，operation按map顺序zip，host手工carrier | candidate typed identity/mapping blocker / T06 | exact admitted mapping与production projection；`e685519` |
| R02@`ee1609c` / F08 | async typed error、callback stream item及callback host合流未闭环 | candidate cross-lane integration blocker / T05 | 复用F06/F07 owner并补full-chain；`9809dee` |
| R02@`9809dee` / F09 | buffered terminal可能阻塞，runtime/producer clone环与异常consumer未统一清理 | candidate stream lifecycle blocker / T05 | terminal/drop状态机与统一consumer cleanup；`484cab0` |
| R02@`484cab0` / F10 | pull source Err不移除registry；file decode/write异常不立即cancel | candidate terminal cleanup blocker / pull/native owner | 共享`StreamConsumerCleanup`覆盖整个消费；`ae7b601`，R02最终PASS |
| R03@`5ba7273` / F11 | Rust string与TS template可伪造checker required anchor/case | checker false-negative / T09 | literal-aware结构识别与mutation覆盖；`b0d144d`，R03三个verdict最终PASS |
| T10@`453c11f` / F12 | 20个旧host tests未建立canonical selector/admitted assembly，在原断言前fail closed | stale fixture / T07 test owner | resolved-spawn test helper迁移，production零修改；`a053d7b` |
| T10@`453c11f` / F13 | typed provider body为空，full-chain只到missing block checkpoint | missing integration evidence / typed fixture owner | 有效provider body与最终业务结果，production零修改；`71e7ab0` |
| T10@`453c11f` / F14 | prepared producer注册到provider runtime，drain/cancel却查interpreter runtime，吞更具体producer error | candidate stream owner regression / T05 | same-handle诊断成立，但单独scope不能协调consumer-first cleanup，报告`TASK_NOT_EXECUTABLE`范围扩张 |
| F14R | F10 cleanup先remove registry，outer owner无法drain尚未运行/pending producer terminal | candidate implementation ownership gap；公共契约不变 | supervised lease、typed End/ProducerError与exact-once handoff；`f093e921` |
| T10@`453c11f` / F15 | 1个Phase Rust fmt差异；command policy漏登记既有spawn；router deps未materialize | mechanical format/policy + gate environment | Phase Rust闭集PASS、policy ledger闭环、frozen installs成功；repo-wide仍只剩3个Phase外fmt baseline |
| T10@`f093e921` / F16 | shared materializer直接解构/重建`WithSource`/`WithDiagnosticFrame`，eval checker 4 DENY | candidate mechanical error-owner blocker，不是typed error语义缺口 | wrapper递归下沉唯一`eval::error` helper，4→0；final runtime重验PASS |
| router selector | 5/384失败均由compiler-generated fixture执行无bin manifest | inherited main baseline，不是Phase 04 regression | 保持FAIL并移交baseline owner；不修改compiler/Cargo/fixture |
| checks selector | package-store discovery下游compiler `cargo run`无bin | inherited main baseline，不是Phase 04 regression | 保持FAIL并移交baseline owner；fail-fast未到项由T10按去重ledger补证 |
| stable首次auto refresh | candidate已编译安装，但health guard拒绝refresh | environment/runtime state | supported `--no-refresh` + canonical `refresh-binaries`最终恢复 |
| stable错误main restart | restart从stable source root重编main并覆盖candidate | process deviation | 四方candidate hash最终恢复；完整记录，不隐去 |
| candidate canonical restart | shared cargo target复用main dependency artifact，4个E0432 | cross-worktree build environment contamination | 未install/stop/start；改用candidate isolated target |
| chat smoke | `/session`无active assembly ingress | `EXPECTED_PHASE05` | 不计Phase 04 PASS；Phase 05迁移后重建live/chat证据 |

所有candidate-native blocker都已由原owner或明确repair owner关闭，并在最终candidate上重建受影响证据。Inherited
baseline与`EXPECTED_PHASE05`没有通过放宽checker、修改fixture、增加compatibility/dual path或伪造PASS来消除。

## 8. Phase 05风险与P4-A01交接

- Stable manifest虽有3个service builds，但runtime registrations为0，Agine `/session`没有active assembly ingress。
  Phase 05必须完成registry/storage/release pointer、authoring/tooling与consumer route迁移后重跑成功chat smoke。
- `compiler/Cargo.toml`无bin与既有`cargo run`调用不一致，导致repo router/checks selector失败。该baseline owner需
  独立收敛；Phase 04不得越界改compiler/Cargo/fixture来制造全绿。
- `instance restart`从执行脚本所在source root构建，且canonical stable config使用共享cargo target。并行worktree
  验证必须避免相同package name/version artifact污染；temp config path又会改变instance identity。后续stable
  验证应沿用candidate isolated build + canonical refresh的supported流程或先明确改进instance工具。
- Runtime仍有少量既有dead-code warning，其中部分owner等待Phase 05 consumer；当前没有warning升级为Phase 04
  schema、call-graph或lifecycle blocker。
- A01应锚定production candidate `13b4600f38ae1d0cdc6878ecb518e2b616d5e4fa` / tree
  `a34e103cb8a95f0611b380ae3a173266471fcc6d`，核对本表真实入口、保留边界、repo baseline失败与
  `EXPECTED_PHASE05`分类。任何production、Cargo、checker、fixture或gate环境变化都要求重新冻结candidate。

T10最终结论：**Phase-scoped requirements/gates无blocker，可提交P4-A01；repo selectors仍有明确inherited
baseline failures，不能把`router`或`checks`命令写成PASS。**
