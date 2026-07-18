# P2-T05：Package-Only Compiler Cutover

状态：rebuild；从 `9ca2547` clean checkpoint 重做。可与 R03/R11 同基线并行开发，但不整体
cherry-pick 旧提交 `9adfd64` 或后续 integration tail。

## 目标

把T02–T04接入唯一production package compiler，删除PublicationInput/Kind、CompiledPublication、
LoweredPublication和package/service option bundle。不得在集成任务重写下游规则。

## 依赖与 worktree

- 依赖 `9ca2547` 中已完成的 T01–T04；与 R03/R11 可从同一基线并行开发，最终先合 R03/R11 再合 T05。
- 建议branch：`codex/package-service-p2-t05-compiler-cutover`。
- 独占 compiler 中央 hot files；旧 runtime/test consumer 任务已取消，不是并行 consumer。
- 2026-07-18 ownership split 后，`compiler/projection/**`、`compiler/emission/**` 由 T05A 独占；本任务
  只定义它们必须消费的 terminal central API，不修改其文件。

## 完成态

1. production入口明确为PackageCompileInput、PackageSourceModel、LoweredPackage、CompiledPackage；类型和
   function名不再以publication作为共同抽象。
2. source/type/lowering只处理package code；service ingress/config/deployment facts不进入canonical source
   model或common projection input。
3. driver只编译 code-free contracts 与 packages；旧 service root 不推导 contract/binding、不经 adapter
   进入 canonical pipeline。
4. T02 effect facts、T03 ServiceCallRefs与T04 PackageArtifact projection完整接线；不新增局部转换规则。
5. 删除PackageProjectionBundle/ServiceProjectionBundle等共同option aggregate；contract producer与package
   producer是两条显式pipeline。
6. compiler facade、rustdoc public API fixture、crate DAG和boundary checker更新到终态；本任务是全部
   compiler structure checker 与 checker self-test的唯一owner。checker负例能捕获production重新
   引入四个旧Publication compiler symbols。
7. 直接触碰的超长driver/source root按input/pipeline拆分，不新建 adapter 目录或 allowlist。
8. 删除 clean base 仍有的旧 service publication/orchestration production 入口及只锁定该语义的 driver
   tests；不修改 runtime/router/test-runner 来弥补断链。

## 写入范围

- compiler input-model、source/compiled/lowering根类型的cutover接线。
- compiler driver pipeline/source_compile、compiler facade及直接tests。
- compiler boundary/DAG/public API checker、checker self-test 与 fixtures；不为兼容路径增加 allowlist。
- `compiler/driver/service_publication_tests.rs` 由本任务唯一处置：package compile 诊断迁到直接 package
  tests，service publication/binding 断言随旧 owner 删除并记录 disposition。
- 不修改T02 effect算法、T03 lowering算法、T04/R03 projection规则或任何 projection/emission 文件。

## 验证

```bash
cargo test -p skiff-compiler-input-model -p skiff-compiler-source -p skiff-compiler-lowering
cargo test -p skiff-compiler-compiled -p skiff-compiler
node scripts/check-compiler-boundaries.mjs
node scripts/check-compiler-crate-dag.mjs
node scripts/check-crate-public-api.mjs --all-configured
git diff --check
```

## 回报

提交commit、自验收矩阵、四个旧compiler type零命中证据和production入口调用图。
发现T02–T04契约缺口时退回owner，不在本任务复制修复。
