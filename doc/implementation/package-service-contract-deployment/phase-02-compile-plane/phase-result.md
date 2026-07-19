# Phase 02 验证结果

状态：STALE；旧T07证据对应`648bc68`之前的候选。A01发现typed contract source/signature主链未闭合，
Phase 02已按phase-plan波次8重新打开；波次9c的R10I为7/7，但独立production复验又发现interface exact
facts与public-instance projection仍有第二owner。T03H/T03I/T04C/T03H1合入后的波次9g候选`1da3545`
在lowering、public-instance与projection聚焦链通过，但R10I因compiler-known `ErrorPayload` owner分类缺口
仅4/7通过；独立F09D同时确认该blocker与std public-path normalization重复owner。T03H2/T04D及复验完成后
候选`2bb5d3e`的R10I/F09D均PASS；波次9j T07的foundation 281/0/1 PASS，但compiler首次总gate暴露并已提交
canonical `/` fixture修复`e3cbffd`，同时发现resolved LocalType debug文本被source exact projector重新parse的
T03J blocker。T03J合入后的候选`3b34570`上R10I/F09D、exact runtime_slots与identity双gate均PASS；boundary
首次运行因checker仍冻结T04A–D之前的public shape与test-only reachability而9-DENY。T05C13完成后只复验
boundary并继续剩余gate，本文旧命令只保留历史证据价值。

## 1. 候选状态

- integration branch：`codex/package-service-phase-02-terminal`。
- 固定 gate 基线：`b6ca45f7682f018bac880e6508ee8b239b67f2fc`（下文记为 `B`）。
- 开始执行时 `HEAD` 精确为 `B`，工作树 clean、无 unmerged path；`9ca2547..B` 的提交链只包含
  Phase 02 terminal compile-plane 交付。旧 T05 integration tail `9adfd64` 不是 `B` 的祖先。
- 最终代码候选 `C` 是 `B` 加 §2 的两项机械修复；result-record commit 是包含本文和该机械
  delta 的提交。本文不记录自指 commit hash，最终提交由 T07 回报给 gate owner。
- 未新增 semantic、wire、identity、artifact、compile pipeline 或 compatibility 行为；未清理或切换
  worktree，未 merge `main`，未 push。

## 2. 机械修复

1. `compiler/projection/src/package_artifact/tests/fixtures.rs` 仍调用已删除 crate
   `skiff_compiler_publication_abi` 的 public-instance operation helper。fixture 现通过 test-only re-export
   复用 projection production 已有的 canonical helper；没有复制 operation identity 规则，也没有恢复
   旧 crate/harness。
2. `PackageCompileError` 的公开 `From<skiff_compiler_projection::ProjectionError>` trait impl 会把内部
   projection crate 暴露到 rustdoc public API。该 impl 改为 `pub(crate)` mapper，并在唯一调用点显式
   `map_err`；variant、message 与错误映射保持逐项不变。

两项均是 P2-T07 允许的 fixture/API 机械修复。没有空/fake contract、旧 service publication harness、
test target 批量删除、legacy adapter、dual path、fallback 或 allowlist 扩张。

## 3. Gate 证据

每个昂贵总 gate 只执行一次。`F` 表示 `B` 加第 1 项 fixture 修复；`C-code` 表示两项修复均已落入
工作树但尚未加入本文。所有命令 owner 均为 P2-T07。

| 层级 | 命令 | 状态 / 耗时 | commit / state | 结果与覆盖 |
| --- | --- | --- | --- | --- |
| foundation | `node scripts/verify.mjs --only foundation` | PASS / 8.04s | `B` | 279 passed、0 failed、1 ignored；canonical-json、artifact-model、artifact-identity、syntax、CLI/integration 与 doc tests |
| compiler | `node scripts/verify.mjs --only compiler` | REPAIRED / 6.61s | `B` | 首次在 projection lib-test 编译期 FAIL；唯一 error 是旧 fixture crate path，未进入测试执行；按 §2.1 修复 |
| compiler repair probe | `cargo test --package skiff-compiler-projection --lib` | PASS / 1.33s | `C-code` | 受影响 crate 编译完成，15/15 tests PASS；证明 canonical helper fixture 修复有效 |
| identity checker self-test | `node scripts/check-artifact-identity-single-source.mjs --self-test` | PASS / 0.06s | `F` | checker 负例与 owner registry 自检 |
| identity structure | `node scripts/check-artifact-identity-single-source.mjs` | PASS / 0.34s | `F` | canonical identity owner、consumer delegation、duplicate/compatibility path 扫描 |
| compiler boundary | `node scripts/check-compiler-boundaries.mjs` | PASS / 0.13s | `F` | terminal compiler source/API boundary；无 known violation |
| compiler crate DAG | `node scripts/check-compiler-crate-dag.mjs` | PASS / 0.31s | `F` | phase 10 policy，17 条 workspace edge |
| rustdoc/public API | `node scripts/check-crate-public-api.mjs --all-configured` | REPAIRED / 12.36s | `F` | `skiff-compiler-contract` PASS；`skiff-compiler` 仅因公开 `From<ProjectionError>` 的 2 条 forbidden reference FAIL，按 §2.2 修复 |
| public API repair probe | `node scripts/check-crate-public-api.mjs skiff-compiler` | PASS / 0.47s | `C-code` | 受影响配置零 forbidden reference；terminal compiler public surface 仅引用 policy allow-list crates |
| targeted rustfmt | `rustfmt --edition 2021 --check -- <9ca2547 以来所有仍存在的 phase Rust 文件>` | PASS / 0.87s | `C-code` | 覆盖 168 个现存 Phase 02 Rust 文件；首次 check 只指出本次 fixture 换行，机械格式修复后 PASS |
| whitespace | `git diff --check` | PASS | `C` | 最终代码与本文；提交前执行 |

未重跑昂贵 compiler 或 all-configured public-API 总 gate。前者的唯一首错由受影响 projection crate
完整 lib test 闭环；后者未受影响的 contract 配置已在总 gate PASS，受影响 compiler 配置由同一 checker
单 crate 模式闭环。最小 workspace check 因此选择上述 projection lib test 与 compiler rustdoc probe，
不再重复相同 cargo graph。

## 4. 结构完成态与反向证据

compiler boundary、crate DAG、public API 与 artifact identity checker/self-test 的组合结果证明：

| 结构要求 | 最终结果 |
| --- | --- |
| production canonical path 中 `PublicationInput` / `PublicationKind` / `CompiledPublication` / `LoweredPublication` owner | 0 |
| `PackageArtifact` / `ServiceContract` 嵌入 `PublicationAbiUnit` / `ServiceUnit` | 0 |
| contract-only consumer path 携带 provider build/deployment/route/executable target | 0 |
| canonical File IR 旧 `ServiceDependencySymbol` producer | 0 |
| compiler production 的 `PublicationAbiUnit` / `PackageUnit` / `ServiceUnit` / `serviceAssembly` producer | 0 |
| compiler production 的 legacy runtime adapter、compatibility/fallback allowlist | 0 |

identity checker 的 self-test 与真实扫描均 PASS，说明归零不是空 checker 或失效 regexp；crate DAG 的
17 条 edge 与 rustdoc allow-list 同时阻止旧 compiler crate owner 经依赖或公开 API 回流。

## 5. Baseline、residual 与暂不可用下游

- 不声明 rustfmt baseline：覆盖全部 phase 现存 Rust 修改文件的 targeted check 已 PASS；未运行 full
  workspace rustfmt，也未把未检查问题标作 baseline。
- foundation/compiler 构建保留 advisory warning：`syntax::Parser::parse_qualified_type_ref` dead code；
  compiler source 另有 unused import/dead-code warnings。现有 gate 不 deny warning，本任务没有据此扩展为
  production cleanup。
- foundation 的 `regenerate_dynamic_build_id_fixture` 仍是一个显式 ignored generator test；不是 skip gate，
  其余 foundation tests 全部执行并通过。
- Phase 02 明确允许旧 runtime、test-runner、router、service CLI/watch/runtime 在转向终态 artifact 前暂时
  不可用；本阶段不为这些下游增加 adapter 或 fallback。
- 未运行 `pnpm verify`、runtime、test-runner、router 或 live selector。特别是 `runtime-live`、
  `db-encrypted-storage-live`、`loop-risk-health-live`、`loop-risk-stress-live` 均属于合同外下游/手工 live
  验证；P2-T07 明确只验 compile-plane，运行它们既不会证明 Phase 02 完成态，也会重复或扩大 gate。

## 6. 证据失效规则

- 修改 `canonical-json`、`artifact-model`、`artifact-identity` 或 `syntax` 会使 foundation 证据失效。
- 修改任一 compiler Rust crate、`compiler/Cargo.toml`、workspace crate membership 或 compiler fixture，会使
  compiler 证据失效；仅本文档变化不失效。
- 修改 identity checker、其 owner registry/self-test、identity derivation/validation consumer，会同时使
  identity self-test 与真实扫描失效。
- 修改 compiler dependency、module/API boundary、crate DAG policy 或 Cargo edge，会使 boundary/DAG 证据
  失效；公开类型、trait impl、re-export 或 public-API policy 变化会使 rustdoc 证据失效。
- blocker 修复只使其直接影响的证据面失效：§2.1 由 projection lib test 重新建立，§2.2 由
  `skiff-compiler` 单 crate public-API probe 重新建立；二者不改变 identity schema、owner registry 或 crate
  edge，因此不使 identity/boundary/DAG 证据失效。
- `C` 之后任何非文档代码变化都必须按上述影响面重新判定；不得用 Phase 02 task-level 旧 commit 证据替代
  受影响的最终 gate。
