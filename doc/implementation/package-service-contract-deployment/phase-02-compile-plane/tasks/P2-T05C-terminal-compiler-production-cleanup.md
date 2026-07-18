# P2-T05C：Terminal Compiler Core Cleanup

状态：checkpoint repair tail；依赖 T05C1/T05C2，可与 T05C3/T05C4 并行，R10 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“核心结论”“不变量”
“Compiler 与 Projection 流水线”“Fail-closed 条件”“非目标”。

## 背景

T05/T05A/T05B 合流后的 structure checker 已正确 fail closed，但 production tree 仍保留旧
publication/service orchestration owner。T05C1/T05C2 已删除 facade/input/model production edge；本任务
负责 core aggregate owner，并物理删除此时真正 orphan 的 publication-ABI crate及 gate 配置。旧 compiler
integration tests 的迁移属于 R10。

## 目标与 ownership

- 物理删除 `compiler/core/**` 及其直接 production 邻接中残留的 PackageUnit/ServiceUnit/
  serviceAssembly consumer/producer。
- 物理删除 orphan `compiler/publication-abi/**`、workspace/Cargo.lock subject 与 checker/verify 配置，
  但保留结构 gate 对旧 publication-ABI edge/symbol 的 deny fixture。
- 独占 compiler core 与 orphan crate/gate-config cleanup；不修改 source/lowering canonical 语义，也不重复
  T05C1/T05C2 的 production实现。

## 完成态

1. `compiler/core/**` 中 structure boundary checker 零 deny；合流前完整 checker 只允许命中
   T05C3/T05C4 明确拥有的 source/lowering 路径，并列出精确结果。
2. publication-ABI crate 不在 filesystem workspace、Cargo metadata/lock、verify subjects或 managed public
   owner中；crate-DAG 对重新引入旧 edge仍 fail closed。
3. 不建立 legacy/compatibility adapter，不恢复 provider inference，不改 artifact wire/identity。
4. `compiler/tests/**`、test-support、Cargo integration test target 与旧 fixture 不在本任务处理；由 R10
   按逐项 disposition 迁移，不能为让测试暂时编译而恢复 production owner。

## 验证

- `cargo check -p skiff-compiler` 及直接受影响 core production crates。
- `node scripts/check-compiler-boundaries.mjs`；合流前仅允许 T05C3/T05C4 写域的精确命中。
- `node scripts/check-compiler-crate-dag.mjs` 及其相关 self-test。
- `cargo metadata` / verify subject 检查证明 publication-ABI crate不再注册。
- production 旧 symbol/owner 反向搜索、targeted rustfmt、`git diff --check`。
- 不运行 compiler integration tests、T07 完整 gate或 runtime/router/test-runner 测试。

提交并保持 worktree clean；回报删除的 production owner、checker 结果、聚焦编译证据与留给 R10 的
测试断链清单。
