# P2-T07：Phase Integration Gate

## 目标

在 R03、R04、R06、R10B、R10C、R10D、R10E、R11、R13 合入，且旧 integration tail 从未进入新 branch ancestry 后，运行一次
阶段gate、修复纯机械fixture、记录精确证据。
不得新增语义或顺手重构。

## 依赖与 worktree

- 直接在Phase02 integration worktree执行，不另建task worktree。
- 依赖 R03、R04、R06、R10B、R10C、R10D、R10E、R11、R13 和所有高风险任务证据。

## 完成态

1. 确认integration候选只包含终态 compile-plane 交付、工作树clean、无未解释冲突。
2. 运行phase-plan指定的foundation/compiler、结构gate和必要workspace check；每个昂贵命令只运行一次。
3. 只允许修复由新canonical wire引起的机械fixture/API拼写；语义失败退回对应 owner。
   旧 service publication harness、空/fake contract 或批量删除 test targets 不属于机械修复。
4. 运行结构反向搜索与checker self-test，证明旧compiler owners和所有compatibility adapter/allowlist归零。
5. 记录`phase-result.md`：commit、命令、owner、结果、覆盖、baseline与证据失效规则。
6. targeted rustfmt覆盖全部phase修改Rust文件；full rustfmt失败必须在main同环境复现才可标baseline。

## Gate

```bash
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only compiler
node scripts/check-artifact-identity-single-source.mjs --self-test
node scripts/check-artifact-identity-single-source.mjs
node scripts/check-compiler-boundaries.mjs
node scripts/check-compiler-crate-dag.mjs
node scripts/check-crate-public-api.mjs --all-configured
git diff --check
```

## 回报

给出最终候选commit、gate表、机械修复、暂时不可用的下游与未运行的live命令及理由。
不得以任务级旧commit证据替代受影响的阶段最终gate。
