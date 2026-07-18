# P2-T05C10F：Identity Checker Terminal Owners

状态：T07 gate blocker；依赖 T05C9 与 terminal compiler cleanup，可与 T05C10D/T05C10E 并行。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“依赖与 Identity”“不变量”。

## 目标与 ownership

- 让 artifact identity single-source checker 只校验终态存在的 canonical owners，不读取已删除的 projection/
  publication identity 文件，也不以跳过缺失文件伪造 PASS。
- 独占 `scripts/check-artifact-identity-single-source.mjs` 及其 self-test/fixture；不修改 Rust production/tests。

## 完成态

1. requirement/owner graph 与当前 terminal artifact-model/artifact-identity/compiler owners 一致；逐项说明删除的旧
   owner 由哪个 canonical owner 取代。
2. 缺失的仍受管 canonical owner 必须 fail closed；已退役 owner 不再进入 requirements。
3. self-test 覆盖 owner 缺失/重复或违规实现，真实 checker 在当前 tree 通过。

## 验证

- checker self-test、真实 checker、`git diff --check`；不运行 Rust/compiler/T07 完整 gate。

提交并保持 worktree clean；回报 owner graph 差异和 checker 结果。
