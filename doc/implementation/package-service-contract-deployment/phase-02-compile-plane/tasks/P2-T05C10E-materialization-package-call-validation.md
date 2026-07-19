# P2-T05C10E：Materialization Package-call Validation

状态：validator consumer；依赖 T05C10C/T05C7，可与 T05C10D/T05C10F 并行，R10 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“Compiler 与 Projection 流水线”
“依赖与 Identity”“Fail-closed 条件”。

## 目标与 ownership

- emission/materialization 在使用 `package_callables` 计算 requirement coverage 前调用 shared validator。
- 独占 `compiler/emission/**` 的接入与直接测试；不得复制 validator 遍历/集合规则。
- 禁止修改 artifact-model、lowering/driver/identity/runtime/checker 或 integration tests。

## 完成态

1. invalid instruction/table pair 在 materialization 前 fail closed，不能只信 table。
2. valid package call requirement coverage 继续只按 `package_ref` 解析，expected ABI 只来自 requirement。
3. missing/orphan/mismatch/duplicate 至少由 shared validator 的 direct tests 与本 consumer 接入测试共同覆盖。

## 验证

- emission/materialization 聚焦 tests、必要 crate check、targeted rustfmt、`git diff --check`。

提交并保持 worktree clean；回报 validator 调用点与 requirement coverage 证据。
