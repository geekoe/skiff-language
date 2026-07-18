# P2-T05C10D：Identity Package-call Validation

状态：validator consumer；依赖 T05C10C/T05C9，可与 T05C10E/T05C10F 并行，R10 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“依赖与 Identity”“Fail-closed 条件”。

## 目标与 ownership

- artifact-identity 在 canonical hashing 前调用 artifact-model 的 package-call validator。
- 独占 `artifact-identity/**` consumer 接入与 mutation tests；不得复制 validator 规则。
- 禁止修改 artifact-model、compiler/runtime、prefix/version 或 checker。

## 完成态

1. missing/orphan/mismatch/duplicate package ref 均拒绝生成 identity。
2. valid File IR v5 identity/golden 不变；service-call validation 不回退。
3. identity 只消费 shared validator，不维护第二套 instruction/table traversal。

## 验证

- artifact-identity package-call/file-ir 聚焦 tests、必要 crate tests、targeted rustfmt、`git diff --check`。

提交并保持 worktree clean；回报 mutation 覆盖。
