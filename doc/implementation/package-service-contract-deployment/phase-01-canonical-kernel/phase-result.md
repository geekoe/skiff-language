# Phase 01 验证结果

状态：P1-T08 PASS，等待 P1-A01 独立只读验收

## 1. 候选状态

- integration branch：`codex/package-service-phase-01`。
- 代码基线候选：`e864ac9`。
- T08 只补了一项机械 fixture 迁移：
  `runtime/driver/program/tests.rs` 的 compiler-shaped PackageUnit JSON 从旧
  `effects: {}` 改为 canonical `effects: { operations: {} }`。第一次 runtime gate 以
  `missing field operations` 稳定暴露该遗漏；focused test 与完整 runtime selector 在修复后均
  PASS。该改动没有改变 production DTO 或运行语义。
- 最终候选 `C` 是 `e864ac9` 加上述 fixture delta；result-record commit is this document's
  commit。本文本身不改变代码门禁结论，因此无需自指提交 hash。
- T08 开始及结果记录前，仓库只剩 `main` 与 phase integration 两个本地分支/worktree；T01–T07
  task worktree 和临时分支均已清理。T08 不 merge `main`，不 push。

## 2. 最终验证证据

`C` 表示上一节定义的最终候选。foundation/compiler 在 `e864ac9` 上运行；之后唯一代码变化是
runtime test fixture，因此两项证据未失效。runtime、router 与最终结构检查均在 `C` 上运行。

| 层级 | 命令 | owner | commit / 状态 | 结果 | 覆盖范围 |
| --- | --- | --- | --- | --- | --- |
| format | `cargo fmt --all -- --check` | T08 | `e864ac9` | BASELINE-ONLY | 只命中仓库已知四个未改文件；见 §3 |
| format replacement | `git diff --name-only --diff-filter=ACMRT -z main...HEAD -- '*.rs' \| xargs -0 rustfmt --edition 2021 --check`，另对 T08 fixture 单文件复跑 | T08 | `C` | PASS | 所有 phase 现存 Rust 改动文件 |
| foundation | `node scripts/verify.mjs --only foundation` | T08 | `e864ac9` | PASS | canonical-json、artifact-model、artifact-identity、syntax 及 CLI/integration tests |
| compiler | `node scripts/verify.mjs --only compiler` | T08 | `e864ac9` | PASS | compiler 分层 crates、facade integration tests 与 doc tests |
| runtime focused | `cargo test -p runtime --lib program::tests::program_units_deserialize_compiler_shaped_ir_json` | T08 | `C` | PASS | canonical effect wire 的旧 fixture 迁移 |
| runtime | `node scripts/verify.mjs --only runtime` | T08 | `C` | PASS | 18 个 runtime packages、runtime lib 与 doc tests |
| router static | `pnpm --filter @skiff/router type-check` | T08 | `C` | PASS | Router TypeScript production/test types |
| router dynamic | `pnpm --filter @skiff/router test` | T08 | `C` | PASS | 24 files、394 tests，含 load/reload、CLI parity 与 tamper cases |
| identity structure | `node scripts/check-artifact-identity-single-source.mjs` | T08 | `C` | PASS | Rust/TS/scripts identity owner、adapter 与 dev-sync path sink |
| identity checker self-test | `node scripts/check-artifact-identity-single-source.mjs --self-test` | T08 | `C` | PASS | duplicate owner、hash/preimage、raw dev-sync path 负例 |
| compiler boundary | `node scripts/check-compiler-boundaries.mjs` | T08 | `C` | PASS | compiler layer/API boundary |
| compiler DAG | `node scripts/check-compiler-crate-dag.mjs` | T08 | `C` | PASS | phase 10，56 workspace edges |
| whitespace | `git diff --check` | T08 | `C` | PASS | 最终代码与本文 |

Router 两项命令第一次启动时因 integration worktree 尚未安装 `router/node_modules` 而在执行
`tsc`/`vitest` 前返回 `ENOENT`；按 `router/pnpm-lock.yaml` 安装本地依赖后，上表两项真实 gate 均
PASS。根目录误生成的空 `pnpm-lock.yaml` 已删除，不属于产品改动。

## 3. rustfmt baseline

full workspace rustfmt 只报告以下四个文件：

- `runtime/capability-context/src/http.rs`
- `runtime/host/src/host/http_runtime/tests/egress.rs`
- `runtime/host/src/host/http_runtime/tests/helpers.rs`
- `runtime/host/src/host/http_runtime/tests/stream.rs`

同一工具链、同一命令在未合入 phase 的 `main` commit `3c2db18` 上逐字复现相同四项 diff；四个文件
均不在 `main...e864ac9` 的 phase diff 中。对 phase 现存 Rust 文件的 targeted rustfmt 以及 T08
fixture 单文件 rustfmt 均 PASS，因此没有把 phase 格式回归计入 baseline。

## 4. 九项阶段完成态

| # | 完成态 | production 代码证据 | 测试 / 反向证据 |
| --- | --- | --- | --- |
| 1 | canonical JSON、framing、hash 与 assign/validate 有模块化单一 owner | `canonical-json/src/lib.rs` 只拥有 JSON canonicalization；`artifact-identity/src/framing.rs` 与按领域拆分的 `file_ir.rs`、`operation.rs`、`package/**`、`publication*.rs`、`semantic.rs` 拥有 identity；`artifact-identity/src/lib.rs` 仅 103 行 facade/re-export | foundation selector PASS；single-source checker 与 self-test PASS；production 中没有第二个 canonical JSON 定义 |
| 2 | nominal type/interface/callable/method identity 全部归 artifact-identity | `artifact-identity/src/semantic.rs` 与 `operation.rs` 拥有 derivation；artifact-model 只保留 typed DTO；compiler 只直接调用或一跳 re-export canonical API | semantic/operation golden 与 compiler tests PASS；旧 `abi_id_key_hex`/字符串 method framing owner 已删除 |
| 3 | PublicationAbi producer/consumer 共用 leaf validator | `artifact-identity/src/publication_validation.rs::{validate_publication_abi_surface, validate_publication_abi_identity}`；package outer validation与 `compiler/input/src/service_dependencies.rs` 的外部 artifact trust boundary 均调用同一 API | tampered identity、duplicate/dangling operation、schema closure tests PASS；compiler service dependency tests PASS |
| 4 | package local ABI/build identity 使用显式 inclusion/exclusion projection | `artifact-identity/src/package/projection.rs::{PackageLocalAbiIdentityProjection, PackageBuildIdentityProjection}`；`package.rs` 同源 assign/validate | package mutation matrix 覆盖 coordinate、nominal、recoverable/effect、implementation/dependency/config/resource/runtime 与排除的 path/provenance/diagnostic；foundation PASS |
| 5 | projection-neutral nominal type closure kernel 被真实 consumer 复用 | `compiler/core/src/type_closure/**` 提供 resolver、walker、cycle guard 与 typed trace；boundary/recoverable adapters 分别位于 `contract/boundary/type_closure.rs`、`recoverable_boundary/type_closure.rs`，spawn 调用同一 boundary policy | compiler selector 的 core/boundary/recoverable/spawn tests PASS；旧 `BoundaryPackageTypeSource`、`RecoverablePackageTypeSource` 均无命中 |
| 6 | PackageUnit production/package-test 只有一条 projection/materialization 路径 | `compiler/projection/src/package_unit_artifacts/**::project_package_ir_artifacts` 是唯一 projection；`compiler/emission/.../package_unit_artifacts.rs::materialize_package_unit_artifact` 是唯一 materializer；package-test 直接委托该 API | production/package-test parity 与 compiler selector PASS；`fn build_package_unit(` 全仓反向搜索归零；旧 typed-artifacts builder 已删除 |
| 7 | effect leaf 显式 Unknown，完整透传且 fail closed | `artifact-model/src/effects.rs` 的 `CallableEffectSummary`、全字段 `CallableMayEffects` 与 fallible `effects_for_boundary`；source/projection-input/runtime projection 使用 typed facts | Unknown round-trip、缺字段/未知字段拒绝、return/throw alias 独立、public callable coverage tests PASS；`SourceEffectMetadata::Empty` 和 placeholder precision 无命中 |
| 8 | ServiceUnit/PackageUnit pointer 与 service assembly/content 跨层严格验证 | `artifact-identity/src/artifact_reference.rs` 的 deny-unknown strict refs 与 `service_artifact_closure.rs::validate_service_artifact_closure`；compiler dependency、runtime loader/host 直接调用；router/scripts 每批只通过 `runtime-program-build-id` CLI，并消费已验证内容 | shared path fixture、CLI tamper/version/protocol tests、runtime loader、router load/reload tests PASS；dev-sync 先 exact-match validated refs，再进入唯一 filesystem owner |
| 9 | 本阶段直接触碰的超长/重复 owner 已按职责拆分，未新增第二规则实现 | identity facade 从原 2550 行级实现缩到 103 行；PackageUnit monolith拆成 projection/export/dependency/metadata/ref modules；boundary/recoverable closure 分别抽到独立 adapter；dev-sync path sink/checker 与 CLI validation adapter分离 | 三项结构 checker PASS；phase-plan 五组 reverse search 无未解释 production duplicate；extra-review 未发现阶段 blocker |

仍然较长的 `contract/boundary.rs`、`recoverable_boundary.rs`、`scripts/skiff-dev-sync.mjs` 保留的是本阶段
非目标的既有 policy/orchestration；本阶段直接触碰的 closure、identity 与 artifact-path 规则已经抽出且由
checker 阻止回流。`check-artifact-identity-single-source.mjs` 超过千行，但主体是声明式 owner 表和负例
self-test corpus；新增 dev-sync source/sink scanner 已拆到独立模块，没有把新解析逻辑继续塞入主 checker。

## 5. 反向搜索说明

| 搜索 | 最终结果与 production 命中解释 |
| --- | --- |
| `rg 'SourceEffectMetadata::Empty\|precision: "placeholder"' compiler artifact-model` | 无命中。测试名中的 `without_placeholder_precision` 不属于该精确 pattern，也不实现 placeholder。 |
| `rg 'fn canonical_json_value\|fn canonical_json_number' artifact-identity compiler runtime` | production 无命中；仅 single-source checker 的负例字符串命中，canonical owner 位于 leaf crate `canonical-json`。 |
| `rg 'BoundaryPackageTypeSource\|RecoverablePackageTypeSource' compiler/projection` | 无命中。 |
| `rg 'fn build_package_unit\(' compiler` | 无命中。canonical API 名为 `project_package_ir_artifacts` 与 `materialize_package_unit_artifact`。 |
| `rg 'serviceAssemblyHashInput\|service_assembly_hash_input' compiler runtime router scripts` | 仅 single-source checker 的禁止 regexp 与 self-test 负例字符串命中；production preimage/hash 无命中。 |
| implementation-links helper/prefix 搜索 | 只命中 `artifact-identity/src/package.rs`、`constants.rs` canonical owner/re-export，以及 checker 的规则/负例；runtime/package-test 无本地 owner。 |
| compiler service dependency 的 legacy index/root callback 搜索 | `indexes/services`、`dynamic_build_id_from_artifact_root`、path-only/`artifactPath` 均无命中；dev/release locator 后统一调用 closure validator。snake_case 字符串只存在于显式拒绝列表。 |
| raw closure path + filesystem sink 搜索 | dev-sync 的递归 `artifactReferencePaths`/反斜杠 normalize collector 已删除。runtime 的剩余 production 命中先完成 closure validation，再把 validated path 解析为 typed `ArtifactRootRelativePath` 并调用 canonical resolver。 |
| strict pointer optional/alias 搜索 | Router types 为必填字段，parser 使用 `readRequiredString`；scripts 使用 exact key allowlist/`requiredString`；Rust refs 为 deny-unknown typed structs。没有 optional pointer 或旧 alias reader。 |

## 6. 已删除的重复/临时路径

- 删除 `compiler/driver/shared/operation_abi_identity.rs` 的 compiler-side identity owner。
- 删除 `compiler/projection/src/typed_artifacts/package_unit.rs` 的第二 PackageUnit builder。
- 将 `compiler/projection/src/package_unit_artifacts.rs` 与
  `compiler/emission/src/emission/package_test_artifacts.rs` monolith 拆为职责模块；package-test 委托
  production materializer。
- 删除 runtime host 的 `assembly_identity.rs`、`projection.rs` 与本地 identity helper，改用
  artifact-identity closure validator。
- 删除 Router `dynamicBuildId.ts` 与 TypeScript service assembly hash/preimage；load/reload 通过单次
  Rust CLI batch 获得 validated contents。
- 删除 compiler service dependency legacy index、snake_case/path-only reader与独立 root build-id callback。
- 删除 runtime/package-test 本地 implementation-links identity/prefix和 production unvalidated fallback。
- 删除 dev-sync recursive artifact path collector；raw closure ref 必须先匹配 CLI validated result，结构
  checker 同时覆盖 alias/bracket access 与 filesystem sink。

## 7. 允许保留的 legacy ledger

| legacy | 当前约束 | 删除阶段 |
| --- | --- | --- |
| `PublicationInput` / `CompiledPublication` / `LoweredPublication` | 只能消费 Phase 01 leaf rules，不新增 service-only analysis | Phase 02 |
| `PublicationAbiUnit` aggregate | identity/builder 只能委托 canonical leaf；不是四目标对象共同父类型 | Phase 03 |
| code-owning `ServiceUnit` / service source compile | 继续承载当前运行路径，不成为新 package/contract 事实 owner | Phase 04 |
| 当前 `serviceAssembly` | identity/closure 已移出；其余 runtime/linker 语义只能按总体计划保留 | Phase 05 退出 semantic owner，Phase 07 删除 tooling adapter |
| production remote relay selection/fallback | 本阶段不改执行语义 | Phase 06 变为不可达，Phase 07 物理删除 |
| 旧 registry、CLI、watch、test-runner 入口 | 只能作为受结构 gate 约束的 consumer，不 dual-read/dual-write | Phase 07 |

没有把未完成的 Phase 01 条款降级成 follow-up。下一步只进入 P1-A01 独立只读验收；若 A01 FAIL，必须
回到对应任务修复并使受影响证据失效，不能直接合并 `main`。
