# P5-D19：Supervisor Log-handle Teardown Audit

## 角色与边界

独立只读audit，输入为`40ed693` / tree `01f6b8d1`及D18记录的次生
`Node FileHandle ... ERR_INVALID_STATE`。与F16A并行，不改源码、不提交、不运行F04/完整source-suite、不操作stable，
不对platform source或F04给verdict。

## 审计目标

- 静态追踪`isolated-test-runtime`、instance supervisor、child exit/stop与日志handle的唯一close owner，确认是否存在
  fire-and-forget close、double close、close后write或立即`process.exit`竞态。
- 使用现有Node test harness或最小真实supervisor失败注入捕获stderr、unhandled rejection与完整stack；探针必须
  在runtime/network前失败、使用临时目录/动态端口并有界清理，禁止重跑昂贵Host fixture。
- 区分主错误保存/传播与cleanup错误；cleanup不得覆盖primary error。若不可复现，报告执行次数、触发点和静态残余
  风险，不凭猜测修改。

输出`AUDIT CLOSED`或`DESIGN GO`：前者需证明现有tests覆盖且异常不可复现/不需阶段repair；后者需冻结精确文件/
symbol、单一幂等可等待close owner、最小测试与写入范围，供新F17任务使用。`skiff-instance.mjs`已>1000行；若需
repair必须提取lifecycle owner，禁止继续内联补丁。
