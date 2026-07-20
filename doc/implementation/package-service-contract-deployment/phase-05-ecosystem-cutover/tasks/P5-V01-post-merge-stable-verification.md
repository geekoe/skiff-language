# P5-V01：Post-Merge Stable Verification

## 角色、顺序与边界

唯一stable/live验证owner，不是开发、gate或独立验收owner。只有P5-A01 PASS且Skiff、
`skiff-packages`、`internals`各自integration branch已恰好一次合入对应`main`后才能执行。
这是Internals `AGENTS.md`要求的post-merge operational verification；不替代pre-merge T13/A01。

不得修改source、tests、checker、fixture、依赖或committed config。环境恢复只使用仓库已有受支持命令并
记录前后状态；若stable对象identity与冻结candidate不一致，或暴露candidate-code blocker，立即停止，
保持worktree/branch并升级给主Agent，不做第二次main merge或现场补丁。

## 前置核对

1. 三仓main commit/tree、merge parent与A01冻结source commit/tree可追溯且production tree
   bit-identical；工作树clean，无在途Agent。
2. stable watch registry、local package store symlink与generated path只指向main；Mongo、router、runtime、
   telemetry进程identity/health已记录。
3. 从Skiff main执行runtime binary build/refresh/restart；从Internals main重建AIHub local packages，
   等待五个deployments在同一个new active generation内完成admission，期间不判断业务结果。

## 唯一live ledger

```bash
cd /Users/geek/workspace/skiff
node scripts/build-dev-runtime.mjs
node scripts/skiff.mjs instance restart .skiff-instance/config.yml runtime

cd /Users/geek/workspace/internals/aihub/service
npm run prepare-packages

cd /Users/geek/workspace/internals
SKIFF_ROOT=/Users/geek/workspace/skiff \
  node scripts/prepare-canonical-assembly.mjs --activate --wait
node skiff-platform/package-registry/registry-phase05-smoke.mjs
node agine/client/e2e/provider-list-smoke.mjs --expect aihub/gpt-5.5

cd /Users/geek/workspace/internals/agine
npm run e2e:chat-smoke
```

`prepare-canonical-assembly --activate --wait`必须断言active assembly/generation、五个deployment closure与
healthy replica registrations；registry smoke必须到达typed publish/resolve/history最终结果；chat smoke必须
先断言provider/list再完成session/create/chat/send/get。

## 输出与收尾授权

第一行`PASS`或`FAIL`。记录三仓main/source commits/trees、runtime binary/PID、active assembly/generation、
replica ids、每条命令exit/耗时/最终业务结果及stable state恢复情况。只有PASS才允许主Agent把Phase 05
标记COMPLETE并删除integration/task worktrees与已合并临时分支；不push。
