# P5-I16：Platform Source Shared-target Combined Probe

## 角色、输入与唯一性

Gate owner只读运行F16A/F16B/F16C合流后的exact clean integration commit/tree；不得编辑、提交、修复或操作stable。
本任务是platform shared-target动态证据与F04原样Host gate的唯一owner。候选和环境不变时，R16与F04 narrow receive
必须复用该证据，不重复完整gate。

开始前记录free space、Cargo.lock blob、端口/进程与source provenance。容量不足时在启动构建前报告BLOCKED；不得
删除用户或其它任务cache。A/B detached worktree必须直接建立在`/Users/geek/workspace`，临时target必须是任务自有
明确路径，结束后清理worktree、端口、进程与任务自有target。

## 验收矩阵

1. 同一exact commit建立路径不同的A/B worktree，共用一个任务自有`CARGO_TARGET_DIR`。从A build
   `skiff-test-runner`，记录binary/rlib hash、mtime与dep-info。
2. 从任意非repo cwd调用B的absolute `scripts/run-skiff-tests.mjs`。B的Cargo必须复用A产物（`Fresh`或hash/mtime
   不变），同时std 11/11与Host 1/1返回exact `provider-observed-helper-mutated`。
3. 在同一任务自有target内仅清理本任务package产物并从B重建，再用A的absolute root运行便宜compile-before-runtime
   探针，证明镜像方向不出现reserved-id；该clean只建立B-origin证据，不能成为步骤2 PASS前置。
4. 聚焦负例：正确platform context下，trust root外复制的`skiff.run/std`仍reserved；missing registry/prelude、
   omitted/relative root及同进程context mismatch均fail closed；canonical symlink正例PASS。
5. production rlib/binary字符串与dep-info没有`compiler/input..std`、`..prelude`或platform用途
   `CARGO_MANIFEST_DIR`；source registry仍唯一std。

任何primary失败立即停止完整gate，不重试掩盖；完整记录首错、阶段、cleanup结果和exact command。FileHandle cleanup
只记录并关联D19，不改变platform verdict。输出PASS/FAIL、commit/tree/lock、A/B路径、target、hash/mtime、Fresh证据、
std/Host计数与资源清理证明。
