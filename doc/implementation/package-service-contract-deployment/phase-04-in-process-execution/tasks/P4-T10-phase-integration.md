# P4-T10：Phase Integration Gate

执行状态：**COMPLETE**。最终冻结production candidate为
`13b4600f38ae1d0cdc6878ecb518e2b616d5e4fa` / tree
`a34e103cb8a95f0611b380ae3a173266471fcc6d`；Phase-scoped requirements/gates无blocker，可提交P4-A01。
Repo `router`与`checks` selector仍分别因main同源的compiler无bin baseline失败，不能写成PASS。完整分层ledger、
stable provenance、失败分类与证据保留边界见`../phase-result.md`。

## 角色、状态与边界

Phase 04唯一昂贵gate owner。权威输入为架构§2、§6–§10、§12、§14、§15，`phase-plan.md`，P4-T01–T09任务合同，
R01–R03 PASS与全部开发证据。

开始前确认所有production owner已合流、无在途写入/设计问题，并建立完成标准→真实入口/动态或结构证据/关键
负例/owner/精确commit覆盖矩阵。缺项时保持pre-acceptance，不得用测试PASS直接冻结。

本任务只做bit-identical integration、机械compile/test seam、唯一gate与result draft；发现语义/owner缺口退回原
任务，不在gate candidate顺手实现。只提交integration branch，不merge main、不push。

## Stability epoch

记录clean candidate commit/tree、Phase 04 baseline、所有task source/merge commit与证据失效边界。candidate冻结后
不改production、Cargo、checker、fixture或gate环境；任何相关修改结束epoch。

## 必验真实链

1. typed provider/consumer artifacts → deployment projection → assembly resolution → typed load/link/admit →
   activation contexts → canonical internal service execution，无手写resolved target、无额外artifact I/O/router frame。
2. ordinary success/typed error、async suspend、stream/cancel、callback/native capability均经过同一kernel；包含所有
   关键fail-closed负例。
3. package direct same-heap mutation与service detached materialization使用同一业务fixture做对照。
4. canonical ingress与internal call命中同一dispatcher；reload generation pin与failed reload保留active。
5. router service caller拒绝且gateway/actor/spawn回归；legacy outbound/remote production reachability归零。

## 唯一gate ownership

先用`node scripts/verify.mjs --list`确认展开，避免重复。最终候选按实际影响运行且只运行一次：

```bash
node scripts/verify.mjs --only runtime
node scripts/verify.mjs --only router
node scripts/verify.mjs --only type-check
node scripts/verify.mjs --only checks
node scripts/check-runtime-crate-dag.mjs
node scripts/check-runtime-artifact-boundaries.mjs
node scripts/check-runtime-execution-boundaries.mjs
git diff --check
```

若`checks`已包含显式checker，不重复执行，以ledger记录展开关系。对所有Phase Rust改动运行targeted rustfmt；对
router改动运行其格式/type-check。运行真实in-process runtime smoke；不得以Phase 05 authoring/adapter作为前置。

按workspace规则，在准备合入main的候选上构建stable runtime并运行`/Users/geek/workspace/internals/agine`的
`npm run e2e:chat-smoke`。Phase 05 consumer尚未迁移导致的精确fail-closed应如实记录为跨阶段预期失败，不能作为
Phase 04 PASS证据；其它失败必须分类。

## 回报

提交候选/result所需commit，回报stable commit/tree、覆盖矩阵、分层ledger
`层级 | 命令 | owner | commit/状态 | 结果 | 覆盖范围`、历史失败分类、保留证据依据、残余Phase 05风险与自验收矩阵。
