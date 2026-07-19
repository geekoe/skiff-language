# P2-T05C10A：Type-only Package Requirement Coverage

状态：checkpoint review blocker；依赖 T05C8，可与 T05C10B/T05C10C 并行，R10 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“依赖与 Identity”“Compiler 与
Projection 流水线”“Fail-closed 条件”。

## 目标与 ownership

- 修复 driver 对内建 std/package dependency 的 used-reference 判断，使 type-only 与 callable 引用共享同一
  canonical requirement closure。
- 独占 `compiler/driver/pipeline/**` 对 package external refs 的消费与直接测试。
- 禁止修改 artifact-model、source/lowering、emission、identity、runtime、checker 或 integration tests。

## 完成态

1. type-only `package_symbols[*].package` 与 direct-call `package_callables[*].package_ref` 都能触发 std requirement。
2. 不读取 `PackageCallableId` 猜 dependency，不恢复 operation symbol table，不增加 fallback。
3. 直接测试覆盖“只引用 std 类型、不调用 std callable”与 callable-only 两条路径。

## 验证

- driver pipeline 聚焦测试、必要 crate check、反向搜索、targeted rustfmt、`git diff --check`。
- 不运行 compiler integration tests 或 T07 gate。

提交并保持 worktree clean；回报两类 reference 的 requirement 证据。
