# P5-F17：Supervisor Log-handle Lifecycle Repair

## 输入与DAG

- 权威设计：`doc/architecture/package-service-contract-deployment.md` §14及阶段标准6；本任务只关闭D18/D19已定位的
  test supervisor资源生命周期，不改变四对象、compile、activation或request语义。
- 输入：D19 `DESIGN GO`；无F16语义依赖。使用新的开发Agent，从包含本合同的exact integration docs commit建立
  `/Users/geek/workspace/skiff-p5-f17-supervisor-lifecycle`、分支`codex/p5-f17-supervisor-lifecycle`。可与F16B/F16C
  并行，完成后是I16硬前置。
- 五分钟内开始实际修改；若必须触碰F16C的isolated/source-suite/verify paths，立即报`TASK_NOT_EXECUTABLE`，不得
  扩张写集。一个clean commit，不merge/push、不操作stable。
- 证据只对exact supervisor/lifecycle module、dedicated test、Node版本与本任务commit不变时有效。

## 写入owner与完成态

owner限`scripts/skiff-instance.mjs`的supervise entry/stop signal接线、新的单一
`scripts/lib/supervised-entry-lifecycle.mjs`及新专用
`scripts/tests/skiff-instance-supervisor-lifecycle.test.mjs`。不得修改`isolated-test-runtime*`、F16 paths、Router、Runtime、
compiler、fixture或manifest/lock。

- 每个supervised child只有一个幂等、可等待的log-handle close Promise；child exit与shutdown并发只关闭一次，所有
  rejection被owner捕获并带component context传播。
- exit handler先从running owner取走entry，再await stdout/stderr close与process-group/pid cleanup；restart只在该
  lifecycle完成且非stopping时调度。
- SIGINT/SIGTERM shutdown设置stopping、取消restart、停止components并await全部entry lifecycle；不用
  fire-and-forget close或立即`process.exit(0)`截断异步cleanup，使用自然退出/`process.exitCode`。
- primary failure与cleanup failure分别保留；cleanup不得覆盖primary。无double close、close-after-exit unhandled
  rejection、timer/process leak或第二lifecycle owner。
- 新专用测试用真实`FileHandle`和真实短命child，把child exit与shutdown交错至少20次；strict unhandled-rejection
  capture断言零`ERR_INVALID_STATE`，主错误标记保留，handles/PID/temp files全部收口。不得用纯action-list double冒充。
- `skiff-instance.mjs`已>1000行；新生命周期必须提取，不继续内联扩张。应用extra-review。

## 唯一聚焦验证

```bash
node --unhandled-rejections=strict --test scripts/tests/skiff-instance-supervisor-lifecycle.test.mjs
node --test scripts/tests/skiff-instance-config.test.mjs
node --check scripts/skiff-instance.mjs
git diff --check
```

不得运行F04/source-suite/完整verify。回报commit/tree/lock、20次交错矩阵、primary/cleanup证据、进程/handle清理、反向
搜索、文件行数与extra-review自验收矩阵。
