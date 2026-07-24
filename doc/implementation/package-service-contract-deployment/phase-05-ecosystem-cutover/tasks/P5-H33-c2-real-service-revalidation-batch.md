# P5-H33：C2 真实 Service 重验批次

状态：Ready

## 父节点链

- 直接父节点：`P5-H32-c1-service-stream-checkpoint-result.md`
- 恢复入口：`P5-H31-r05-batch-handoff.md`
- 两者均向上追溯到唯一权威设计。

## 批次目标与 owner

- Registry（skiff-packages）：20/20 intended operations Available，真实 immutable record/pointer storage。
- Codex Relay（Internals）：17/17 intended operations Available，30 routes 与公开 handler 精确对应。
- Account（Internals）：修复旧式调用地址，验证 21 routes。
- AIHub（Internals）：区分 interface/instance/internal/public executable，闭合 managed LLM stream 与 dev config。
- Agine（Internals）：零普通 service-call operation 可合法；HTTP/WebSocket ingress 与 service-call API 独立，并验证
  AIHub stream consumer。

## 批次规则

- 每个叶子只写自己的 service 目录及必要的同 service workflow fixture；不得修改共享 compiler/runtime 或其他 service。
- Linked worktree 只使用隔离的 temporary ecosystem store 和显式
  `SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration`；不得 build/dev/start、注册 stable watch、reload stable
  或写 stable artifact root。
- 每个任务先确认真实 selector/命令可执行；零测试、只检查生成 JSON 或只编译 provider 都不算真实验收。
- 若共同上游缺口出现，返回精确 blocker，由主 Agent 提升共享 checkpoint。
- 本批次不运行完整 Phase gate，不 push、不操作 stable。

