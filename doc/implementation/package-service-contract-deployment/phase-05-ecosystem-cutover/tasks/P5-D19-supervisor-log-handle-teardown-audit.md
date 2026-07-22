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
