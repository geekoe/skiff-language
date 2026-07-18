# P2-T05C：Terminal Compiler Core Cleanup

状态：checkpoint repair split；依赖 R13，可与 T05C1 并行，二者都必须在 R10 fixture 迁移前完成。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“核心结论”“不变量”
“Compiler 与 Projection 流水线”“Fail-closed 条件”“非目标”。

## 背景

T05/T05A/T05B 合流后的 structure checker 已正确 fail closed，但 production tree 仍保留旧
publication/service orchestration owner。facade/input/publication-ABI 表面由 T05C1 并行清理；本任务
负责 R13 合流后的 core 残留。完整 production gate 在二者合流后运行；旧 compiler integration tests
的迁移属于 R10。

## 目标与 ownership

- 物理删除 `compiler/core/**` 及其直接 production 邻接中残留的 PackageUnit/ServiceUnit/
  serviceAssembly consumer/producer。
- 独占 R13 后的 compiler core terminal cleanup 与最终 production structure verification；不修改
  source/lowering canonical 语义，也不重复 T05C1 的 facade/input/Cargo cleanup。

## 完成态

1. `compiler/core/**` 及本任务直接 production 邻接中 structure boundary checker 零 deny；合流前完整
   checker 只允许命中 T05C1/T05C2 明确拥有的路径，并列出精确结果。
2. 不通过 core 特例重新引入 T05C1 正在删除的 facade/input/Cargo/DAG 旧 owner。
3. 不建立 legacy/compatibility adapter，不恢复 provider inference，不改 artifact wire/identity。
4. `compiler/tests/**`、test-support、Cargo integration test target 与旧 fixture 不在本任务处理；由 R10
   按逐项 disposition 迁移，不能为让测试暂时编译而恢复 production owner。

## 验证

- `cargo check -p skiff-compiler` 及直接受影响 core production crates。
- `node scripts/check-compiler-boundaries.mjs`；合流前仅允许 T05C1/T05C2 写域的精确命中。
- crate DAG 由 T05C1 验证；本任务只反向检查 core 不新增旧 edge。
- production 旧 symbol/owner 反向搜索、targeted rustfmt、`git diff --check`。
- 不运行 compiler integration tests、T07 完整 gate或 runtime/router/test-runner 测试。

提交并保持 worktree clean；回报删除的 production owner、checker 结果、聚焦编译证据与留给 R10 的
测试断链清单。
