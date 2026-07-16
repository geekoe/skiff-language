# P1-T12：Phase 01 集成与 Gate

状态：`ready`
类型：阶段集成
启动依赖：无；完成依赖：P1-T00 至 P1-T11 全部完成
执行者：阶段集成 Agent，一份集成/证据提交

## 目标

在任何开发任务前创建并持续独占Phase 01 integration worktree。按DAG逐批合并、每批发布唯一
checkpoint commit供下游建branch；最后修复纯集成问题、运行阶段gate并留下A01验收证据。此任务
不是补做遗漏feature的兜底节点，也不是到T11结束后才第一次运行。

## 合并顺序

1. T00、T02、T05、T06（同批无相互依赖）。
2. T01（需基于已合入T00的checkpoint，避免两个架构任务并发修改同一canonical文档）。
3. T03、T07。
4. T04、T08。
5. T09。
6. T10。
7. T11。

每批合并后至少运行 `cargo check` 到被合并 crate；不要等全部 merge 后才发现接口偏差。
协调Agent记录C0…C7的commit hash并明确通知下一批Agent；任务Agent不得自行cherry-pick多个
dependency branch。若同批任务有机械冲突，由协调Agent解决；若语义冲突，退回原任务。

## 允许的修复

- import/re-export、fixture builder、新 required field 的调用点补齐；
- 多任务独立新增模块后的命名/visibility 冲突；
- deterministic ordering 或 identity golden 的明显接线错误；
- 阶段文档中的实际测试命令/文件路径更新。

不允许：

- 新增 effect/boundary/resolver 语义；
- 保留 dual reader/writer、stub 或 fallback“先让 gate 过”；
- 把失败测试删除而不按 `test-disposition.md` 说明；
- 合并职责已拆开的文件。

## 集成审计

使用 `rg` 和 diff 主动检查：

- service requirement parser/resolver 是否只有一个 owner；
- identity/hash 是否只在 artifact-identity 实现；
- boundary availability 是否只由 package boundary projector生成；
- 是否仍有 package 无条件拒绝 `services` 的 guard；
- 是否仍将正式 effect 固定为 `Empty`；
- 新增/实质修改的 production 文件是否超过约 800 行或核心函数超过约 150 行；
- package/service 是否出现复制分支；
- 是否意外引入 provider package/build id寻址、router fallback 或用户可见 stub。

发现语义缺口时退回对应任务或新增前置任务，不能在本任务顺手实现。

## 阶段 Gate

```bash
cargo test --no-fail-fast \
  -p skiff-artifact-model \
  -p skiff-artifact-identity \
  -p skiff-compiler-input \
  -p skiff-compiler-compiled \
  -p skiff-compiler-projection-input \
  -p skiff-compiler-projection \
  -p skiff-compiler-publication-abi
node scripts/verify.mjs --only compiler
cargo test --no-fail-fast -p skiff-runtime-package-test -p skiff-test-runner
node scripts/check-compiler-boundaries.mjs
node scripts/check-compiler-crate-dag.mjs
node scripts/check-artifact-identity-single-source.mjs
git diff --check
```

不跑 `pnpm test`、`pnpm verify`、live instance 或 chat smoke。

## 证据

创建或更新 `phase-01-code-contract-foundation/integration-evidence.md`，并记录：

- 合并的 task commit列表；
- 每条 gate 的命令、结果、耗时；
- `test-disposition` 中 delete/rewrite 的实际清单；
- 阶段验收七个样例各自对应的 test/fixture；
- duplication/长文件审计结果；
- 已知 non-blocking 限制，仅限 Phase 02+ 明确接手事项。

## 验收标准

- 所有 gate 通过，无 skip/xfail掩盖新失败。
- Phase 01完成态逐项有测试或静态证据。
- production path无新双 owner、兼容 reader或 fallback。
- integration branch可进入只读独立验收。

## 停止条件

- 任一 task 实际输出偏离其 contract；
- gate 失败暴露新语义缺口；
- 为合并必须选择一个文档未决定的架构行为；
- coherent state 只能靠保留两条 production path的新规则实现。

## 提交

即使不需要代码修复，也提交 `integration-evidence.md`；有修复时放在同一集成提交。提交信息
建议：`chore(compiler): integrate package code contract foundation`
