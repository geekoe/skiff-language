# Phase 02 验证结果

状态：PASS。Phase 02 terminal compile-plane、T03A–J、T03H1–H2、T04A–D、T05C13、R10H已经合流；
同一production候选上的R10I为7/7 PASS，F09D为PASS。T07按恢复合同只运行每个昂贵总gate一次，并以受影响的
最小probe闭环canonical dependency fixture、resolved exact-type projection、boundary checker与public API机械问题。

## 1. 候选状态

- integration branch：`codex/package-service-phase-02-terminal`。
- Phase 02 rebuild基线：`9ca2547aea9bd9f7ec787c39c8c81b0fddd6d099`；旧integration tail `9adfd64`
  从未进入当前branch ancestry。
- T07首次总gate候选：`2bb5d3edc6938a81a54dddbc80cbb8c899de4b4f`，开始时工作树clean、无unmerged
  path；foundation与compiler总gate只在该候选运行一次。
- 恢复checkpoint：`b9835d75b6154c1233af1dd04e5e7fe64f1a66cf`（下文记为`B`），tree为
  `94faac5e49a5d7d3e2acc574d2611490f20f4394`，与T05C13开发提交`a6b4cf6`的tree完全相同；恢复时
  工作树clean。
- 最终代码状态`C-code`是`B`加§2.2–2.3两项T07机械修复。result-record commit同时包含本文；本文不记录
  自指commit hash，最终hash由T07回报给gate owner。
- 未merge `main`、未push，未新增legacy/compatibility adapter、dual path、fallback或allow-list扩张。

## 2. 修复时间线

### 2.1 Canonical dependency fixture（已提交`e3cbffd`）

首次compiler总gate发现`compiler/tests/package_imports.rs`与
`compiler/tests/provider_connect_packages.rs`仍用旧`dependency.operation(...)`fixture。它们机械迁移为
canonical `dependency/operation(...)`，folded public-path负例同步断言新的source-resolution诊断。

修复没有改变production parser、resolution或lowering语义。`package_imports`首次repair probe已有6/7通过，唯一
剩余项只是旧诊断文本；更新断言后的exact probe通过最后1项。`provider_connect_packages`为3/3 PASS。

### 2.2 Facade public API mapper（本次result commit）

`PackageCompileError`上的derived public `From<skiff_compiler_compiled::ProjectionInputBuildError>`会把内部compiled
crate暴露进facade rustdoc。移除`#[from]`，改为crate-private `projection_input_error` mapper，并在唯一pipeline
调用点显式`map_err`。错误variant、source、display message与成功路径均不变；没有改变compiled fallible handoff。

### 2.3 Fixture rustfmt（本次result commit）

220文件targeted rustfmt只发现`compiler/tests/service_conformance.rs`一个raw string参数缩进。按rustfmt唯一delta
机械修正，并用受影响单文件probe闭环；fixture字符串内容和R10I typed assertions均未变化。

### 2.4 非机械owner修复

- T03J提交`3b34570`删除`ResolvedTypeRef` debug/display文本回流source parser的路径，以完整
  `PackageTypeRef` sidecar保留Map keys/for-in派生类型与contract identity；T07只运行合同指定exact probe。
- T05C13在与`B` bit-identical的tree上刷新terminal boundary checker：按真实`#[cfg(test)]`可达性排除test-only
  lowering fixture，并精确冻结T04A–D public shape；没有修改Rust production或增加宽allow-list。

## 3. Gate证据

所有命令owner均为P2-T07，只有标明T05C13/R10I/F09D的行使用对应独立owner证据。总gate失败后的修复只运行
受影响最小probe，不重跑foundation、compiler、boundary或all-configured public-API总gate。

| 层级 | 命令 | 状态 / 耗时 | commit / state | 结果与覆盖 |
| --- | --- | --- | --- | --- |
| R10I prerequisite | `service_conformance`及合同内最小依赖检查 | PASS | T03J后的同一production候选 | 7/7；真实provider/consumer source只凭同一ServiceContract分别编译，typed反向断言无provider/deployment读取 |
| F09D prerequisite | lowering全crate、compiled public-instance handoff及最小projection probes | PASS | T03J后的同一production候选 | source/interface/execution/public-instance/public-path owner链独立production复验 |
| foundation | `node scripts/verify.mjs --only foundation` | PASS / 3.14s | `2bb5d3e` | 281 passed、0 failed、1 ignored；canonical-json、artifact-model、artifact-identity、syntax及doc tests |
| compiler total | `node scripts/verify.mjs --only compiler` | REPAIRED / 22.53s | `2bb5d3e` | `--no-fail-fast`完成全部target；仅3个target共6项失败：5项canonical `/` fixture与1项T03J exact projection blocker |
| package fixture probe | `cargo test --package skiff-compiler --test package_imports` | REPAIRED / 1.61s | fixture working state | 6/7 PASS；唯一失败为folded-path旧诊断断言 |
| package fixture exact | `cargo test --package skiff-compiler --test package_imports public_path_shape_is_preserved_under_dependency_alias -- --exact` | PASS / 0.86s | `e3cbffd`内容 | 最后1项PASS；与上一行合成final 7/7覆盖 |
| provider fixture probe | `cargo test --package skiff-compiler --test provider_connect_packages` | PASS / 0.67s | `e3cbffd`内容 | 3/3 PASS |
| T03J repair probe | `cargo test -p skiff-compiler --test runtime_slots map_keys_and_for_in_lower_to_typed_slots -- --exact --nocapture` | PASS / 6.13s | `3b34570` | 1/1 PASS；Map keys、单/双binding for-in与LocalType exact sidecar闭环 |
| identity self-test | `node scripts/check-artifact-identity-single-source.mjs --self-test` | PASS / 0.16s | `3b34570` | registry-derived owner/consumer负例与duplicate mutation自检 |
| identity structure | `node scripts/check-artifact-identity-single-source.mjs` | PASS / 0.38s | `3b34570` | canonical identity owner、consumer delegation、legacy/compatibility与package-call validator扫描 |
| boundary initial | `node scripts/check-compiler-boundaries.mjs` | REPAIRED / 0.13s | `3b34570` | 9 DENY均为test-only可达性或T04A–D已验收terminal shape未同步，交T05C13 owner修复 |
| boundary final | checker `--self-test`、checker tests、production checker | PASS / owner证据 | tree `94faac5e` | T05C13 self-test 11 cases、checker tests 10/10、production零DENY；`B`与owner tree bit-identical，T07不机械重跑 |
| compiler crate DAG | `node scripts/check-compiler-crate-dag.mjs` | PASS / 0.40s | `B` | phase 10 policy，17条workspace edge |
| public API total | `node scripts/check-crate-public-api.mjs --all-configured` | REPAIRED / 4.61s | `B` | `skiff-compiler-contract` PASS；`skiff-compiler`仅derived public `From<ProjectionInputBuildError>`产生2条forbidden reference |
| public API repair probe | `node scripts/check-crate-public-api.mjs skiff-compiler` | PASS / 0.36s | `C-code` | facade零forbidden reference；terminal public surface只引用policy allow-list crates |
| targeted rustfmt | `rustfmt --edition 2021 --check -- <9ca2547以来所有仍存在的phase Rust文件>` | REPAIRED / 0.92s | `C-code` | 覆盖220个Rust文件；唯一delta为§2.3 raw-string缩进 |
| rustfmt repair probe | `rustfmt --edition 2021 --check -- compiler/tests/service_conformance.rs` | PASS / 0.03s | `C-code` | 受影响fixture格式闭环 |
| whitespace | `git diff --check` | PASS | final result working state | 覆盖机械代码、fixture和本文；提交前唯一执行 |

## 4. 终态结构证据

| 完成态要求 | 证据与结果 |
| --- | --- |
| `PublicationInput` / `PublicationKind` / `CompiledPublication` / `LoweredPublication` production owner | boundary零DENY，归零 |
| `PackageArtifact` / `ServiceContract`嵌入`PublicationAbiUnit` / `ServiceUnit` | identity owner扫描与typed wire tests，归零 |
| compiler production的`PublicationAbiUnit` / `PackageUnit` / `ServiceUnit` / `serviceAssembly` producer | boundary与identity扫描，归零 |
| contract-only consumer携带provider build/package/deployment/route/executable target | R10I 7/7 typed反向断言，归零 |
| lowering旧contract operation index与callee字符串回读 | F09D与terminal boundary，归零 |
| dependency call dot compatibility与旧AST owner | canonical `/` compiler diagnostics、fixture repair与F09D，归零；`.`仅保留qualified type/member语义 |
| contract executable/interface从File IR `ServiceSymbol`或display文本重建exact type | F09D、T03J及opaque execution tests，归零 |
| projection blanket Local、public-instance `OperationAbiRef`/File IR semantic signature owner | R10I/F09D与boundary，归零 |
| compiled/projection-input从File IR/`TypeResolutionModel`重算interface conformance | F09D与T05C13 frozen shape，归零 |
| source-declared、typed package、compiler-known与invalid interface分类 | R10I/F09D PASS；单一canonical owner分类，invalid fail closed |
| std public-path normalization | T04D/F09D与boundary frozen helper证明唯一production owner |
| LocalType debug文本回流parser、Map keys/for-in exact sidecar丢失 | T03J exact probe PASS，归零 |
| legacy runtime adapter、compatibility/fallback allow-list | identity self-test/真实扫描与boundary mutation tests，归零 |

## 5. Baseline、residual与未运行项

- 不声明rustfmt baseline：220文件targeted check的唯一格式差异已机械修复并由单文件probe闭环。未运行full
  workspace rustfmt，也未把未检查问题标为baseline。
- foundation/compiler保留非deny advisory warning：`syntax::Parser::parse_qualified_type_ref` dead code，以及
  compiler source既有unused import/dead-code warning；T07不据此扩大production cleanup。
- foundation的`regenerate_dynamic_build_id_fixture`是显式ignored generator test；其余foundation tests均执行。
- Phase 02允许旧runtime、test-runner、router、service CLI/watch/runtime在后续阶段转向terminal artifact前暂时
  不可用；本阶段没有为其增加adapter或fallback。
- 未运行`pnpm verify`、runtime、test-runner、router及live selector。`runtime-live`、
  `db-encrypted-storage-live`、`loop-risk-health-live`、`loop-risk-stress-live`均为合同外下游/手工live验证，不能
  证明compile-plane完成态，运行会扩大或重复gate。

## 6. 证据失效规则

- 修改`canonical-json`、`artifact-model`、`artifact-identity`或`syntax`使foundation证据失效；当前T03J、T05C13
  及T07机械修复均未触及该范围。
- 修改任一compiler Rust crate、`compiler/Cargo.toml`、workspace membership或compiler fixture，必须按影响面
  重新判断compiler复合证据；改变source exact projection或`runtime_slots`fixture会直接使T03J probe失效。
- 修改typed contract production owner、R10 common/service fixture、ServiceContract/PackageArtifact schema或
  compiler success pipeline会使R10I失效；修改source/lowering/compiled/projection-input/projection owner链会使
  F09D失效。§2.2只收窄错误转换API，§2.3只改Rust缩进，均不改变两项动态/production证据。
- 修改identity checker、owner registry/self-test、identity derivation或validation consumer，会同时使identity
  self-test与真实扫描失效。
- 修改boundary checker、terminal public-shape registry、compiled/projection-input frozen API或compiler module
  import边界，会使T05C13 boundary证据失效；test-only排除必须继续由真实`#[cfg(test)]`可达性派生。
- 修改Cargo dependency/workspace edge使DAG证据失效；修改公开类型、trait impl、re-export或public-API policy使
  rustdoc证据失效。§2.2的受影响rustdoc面已由单crate probe重新建立，未改变Cargo edge。
- 修改任一Phase Rust文件使对应targeted rustfmt覆盖失效；result commit之后任何代码或文档变化都使最终
  `git diff --check`失效。不得用任务级旧commit证据替代受影响的最终证据面。
