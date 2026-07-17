# P2-T05：Package-Only Compiler Cutover

## 目标

把T02–T04接入唯一production package compiler，删除PublicationInput/Kind、CompiledPublication、
LoweredPublication和package/service option bundle。不得在集成任务重写下游规则。

## 依赖与 worktree

- 依赖T02、T03、T04全部合入integration checkpoint。
- 建议branch：`codex/package-service-p2-t05-compiler-cutover`。
- 可与T06并行，独占compiler中央hot files。

## 完成态

1. production入口明确为PackageCompileInput、PackageSourceModel、LoweredPackage、CompiledPackage；类型和
   function名不再以publication作为共同抽象。
2. source/type/lowering只处理package code；service ingress/config/deployment facts不进入canonical source
   model或common projection input。
3. driver先编译contracts与packages；legacy service root通过显式typed adapter构造package coordinate/input，
   只调用一次canonical pipeline。
4. T02 effect facts、T03 ServiceCallRefs与T04 PackageArtifact projection完整接线；不新增局部转换规则。
5. 删除PackageProjectionBundle/ServiceProjectionBundle等共同option aggregate；contract producer与package
   producer是两条显式pipeline。
6. compiler facade、rustdoc public API fixture、crate DAG和boundary checker更新到终态；checker负例能捕获
   production重新引入四个旧Publication compiler symbols。
7. 直接触碰的超长driver/source root按input/pipeline/adapters拆分，legacy adapter有Phase03删除owner。

## 写入范围

- compiler input-model、source/compiled/lowering根类型的cutover接线。
- compiler driver pipeline/source_compile、projection facade、compiler facade及直接tests。
- compiler boundary/DAG/public API checker和fixtures。
- 不修改T02 effect算法、T03 lowering算法、T04 projection规则或T06 test-runner/runtime adapter。

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

提交commit、自验收矩阵、四个旧compiler type零命中证据、production入口调用图及legacy service只编译一次
的测试。发现T02–T04契约缺口时退回owner，不在本任务复制修复。
