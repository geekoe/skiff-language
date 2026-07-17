# P2-T07：Phase Integration Gate

## 目标

在T05/T06合入后的稳定候选上运行一次阶段gate、修复机械fixture、记录精确证据。不得新增语义或顺手重构。

## 依赖与 worktree

- 直接在Phase02 integration worktree执行，不另建task worktree。
- 依赖T05、T06和所有高风险任务证据。

## 完成态

1. 确认integration候选包含T01–T06精确commits、工作树clean、无未解释冲突。
2. 运行phase-plan指定的foundation/compiler/test-runner/runtime及三个结构gate；每个昂贵命令只运行一次。
3. 只允许修复由新canonical wire引起的机械fixture/API拼写；语义失败退回对应T01–T06 owner。
4. 运行结构反向搜索与checker self-test，证明旧compiler owners归零且legacy adapter没有扩散。
5. 记录`phase-result.md`：commit、命令、owner、结果、覆盖、baseline与证据失效规则。
6. targeted rustfmt覆盖全部phase修改Rust文件；full rustfmt失败必须在main同环境复现才可标baseline。

## Gate

```bash
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only test-runner
node scripts/verify.mjs --only runtime
node scripts/check-artifact-identity-single-source.mjs --self-test
node scripts/check-artifact-identity-single-source.mjs
node scripts/check-compiler-boundaries.mjs
node scripts/check-compiler-crate-dag.mjs
node scripts/check-crate-public-api.mjs --all-configured
git diff --check
```

## 回报

给出最终候选commit、gate表、机械修复、未运行的live命令及理由。不得以任务级旧commit证据替代受影响的
阶段最终gate。
