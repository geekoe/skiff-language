# P2-T05C10B：Package-call Lowering Fail-closed

状态：checkpoint review blocker；依赖 T05C4/T05C9，可与 T05C10A/T05C10C 并行，R10 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“Package direct call”“Compiler 与
Projection 流水线”“Fail-closed 条件”。

## 目标与 ownership

- 已知 package dependency root 的 call 必须由 typed `ResolvedCallTargetFacts` 精确解析；缺失、Unknown 或
  不匹配时在 lowering 报错，绝不能退化为 `ExternalServiceSymbol`。
- 独占 `compiler/lowering/**` 对 dependency call target 的判定、直接测试与 File IR v5 lowering golden。
- 禁止修改 source fact shape、artifact schema、driver/emission/identity/runtime/checker 或 integration tests。

## 完成态

1. 已解析 package call 只生成 `PackageCallable`；已知 dependency 的 missing/Unknown target fail closed。
2. symbol-path fallback 只处理不属于已知 package dependency 的路径。
3. 直接测试覆盖 valid/missing/Unknown，且 canonical identity golden 唯一使用 v5。

## 验证

- lowering 聚焦测试和 crate check、旧 fallback 反向搜索、targeted rustfmt、`git diff --check`。
- 不运行 compiler integration tests 或 T07 gate。

提交并保持 worktree clean；回报错误边界与 v5 golden。
