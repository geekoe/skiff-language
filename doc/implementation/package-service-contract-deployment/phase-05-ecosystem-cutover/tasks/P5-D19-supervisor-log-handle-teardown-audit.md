# P5-D19：Supervisor Log-handle Teardown Audit

## 角色与边界

新的独立只读audit Agent，输入为`40ed693` / tree `01f6b8d1`及D18记录的次生
`Node FileHandle ... ERR_INVALID_STATE`。与F16A并行，不改源码、不提交、不运行F04/完整source-suite、不操作stable，
不对platform source或F04给verdict。
权威设计仍为`doc/architecture/package-service-contract-deployment.md` §14及阶段标准6；本任务只审计证据owner与
test-supervisor cleanup，不改变production runtime语义。

## 审计目标

- 静态追踪`isolated-test-runtime`、instance supervisor、child exit/stop与日志handle的唯一close owner，确认是否存在
  fire-and-forget close、double close、close后write或立即`process.exit`竞态。
- 运行下列唯一现有baseline测试并明确它只是cleanup顺序double，不是real-handle关闭证据：

  ```bash
  node --test --test-name-pattern='partial supervisor startup failure' scripts/tests/isolated-test-runtime.test.mjs
  rg -n 'void current\?\.(out|err)\.close|process\.exit\(0\)|child\.on\(.exit' scripts/skiff-instance.mjs
  ```

  不创建临时production probe，不运行真实runtime/network或Host。
- 区分主错误保存/传播与cleanup错误；cleanup不得覆盖primary error。若不可复现，报告执行次数、触发点和静态残余
  风险，不凭猜测修改。

当前baseline没有real-handle test，因此`AUDIT CLOSED`不是允许输出。第一行只给`DESIGN GO`或`AUDIT BLOCKED`；
`DESIGN GO`必须冻结精确file/symbol、单一幂等可等待close owner、primary/cleanup传播、真实FileHandle+短命child至少
20次交错测试及F17不重叠写集。`skiff-instance.mjs`已>1000行，repair必须提取lifecycle owner，禁止继续内联补丁。

## 结果ledger

`DESIGN GO`于`f15c210` / tree `b07de45`完成，production相关blob相对D18 `40ed693`未变。baseline double为
`1 pass / 0 fail`，但没有ChildProcess/FileHandle/exit/rejection，只证明上层cleanup顺序。

静态owner确认：`startManagedProcess`打开真实stdout/stderr handles；supervised child exit先删除running entry，再
fire-and-forget两个`close()`及group/PID async cleanup；restart不等log close。`stopAll`只等managed process stop后立即
`process.exit(0)`，signal handler又丢弃`stopAll` Promise。isolated caller已正确保留primary并在cleanup失败时按
primary-first聚合，故F17不得修改它。

F17唯一owner/API、acquisition handoff、restart/shutdown顺序和20轮交错矩阵已写入F17合同；写集不触及F16C路径。
本审计未编辑源码、启动runtime/Host/stable或给F04 verdict。
