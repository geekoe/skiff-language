# P2-T05C9：File IR Identity Version

状态：identity consumer migration；依赖 T05C6，可与其它 consumers并行，R10 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“不变量”
“Package direct call”“依赖与 Identity”“Fail-closed 条件”。

## 目标与 ownership

- 让 artifact-identity 对 T05C6 的 File IR v5/v3 wire生成唯一 v5 identity。
- 独占 `artifact-identity/**` 的 File IR prefix、canonical hashing direct tests与goldens。
- 禁止修改 artifact-model、compiler、runtime、scripts/checker 或 compatibility reader。

## 完成态

1. `FILE_IR_IDENTITY_PREFIX` 为`skiff-file-ir-v5:sha256`；artifact-identity production/tests中旧v4 prefix
   零命中，不接受或生成双prefix。
2. canonical File IR golden hash按新wire重算；mutation matrix继续证明package call target/ref字段进入identity。
3. 其它artifact identity prefix/algorithm不变，不因display string相同复用identity。

## 验证

- artifact-identity全crate tests、self-test（若有）、反向搜索、targeted rustfmt、`git diff --check`。
- 不运行compiler/runtime/T07 gate。

提交并保持worktree clean；回报prefix/golden变化、identity coverage与consumer handoff。
