# P5-D82：普通 Service Call Stream 能力审计

状态：Ready（只读）

## 权威设计

- `doc/architecture/package-service-contract-deployment.md`
- 相关条款：ServiceContract/ServiceDeployment 由 compiler 生成；Service API 类型复用 Package 类型机制；
  service call 通过生成 contract operation、精确 dependency selector、Router/Runtime wire 执行。

## DAG 与目的

- 节点：C1 shared service-call stream capability audit。
- 前置：当前 Phase 5 integration checkpoint；不依赖 P5-F136 的 golden 数值。
- 目的：确认普通 service-call 的 generic/stream contract、compiler lowering、wire、Runtime materialization 是否已有
  canonical owner，并把缺口拆成最短共享 checkpoint；AIHub managed LLM stream 仅作为首个真实 consumer 证据。
- 本任务只读，不给出阶段 verdict，不修改代码或文档。

## 审计范围

- Skiff integration：artifact model/identity、compiler lowering、runtime boundary/eval/recoverable、Router/runtime protocol、
  test runner 与现有 service-call tests。
- Internals integration：AIHub `api.yml`、managed LLM stream declaration/implementation/call sites。
- 区分 HTTP `HttpResponseStreamEvent` 与普通 service-call stream；前者存在不能证明后者完成。

## 必须返回

1. 从 AIHub 真实 public callable 到 compiler artifact、wire dispatch、Runtime value materialization 的关键跳点与文件/符号。
2. 每个跳点状态：complete、missing、duplicate owner 或 blocked-by-upstream；说明上游遮挡。
3. generic 类型实例化、stream item/error/end lifecycle、取消/断连、detached lifetime 的现有语义证据。
4. 最小正/负诊断探针和已有测试；不得运行完整 gate。
5. 判断：
   - `READY_TO_IMPLEMENT`：设计已明确，给出非重叠写入 owner 和建议 DAG；或
   - `DESIGN_BLOCKED`：精确列出会改变公共类型、stream lifecycle 或错误语义的最小用户决策。
6. 搜索证明不存在把 HTTP stream 误当 service-call stream 的 fallback/adapter。

## 证据边界

- 风险：高（共享 contract/wire/lifecycle）。
- 证据只对审计时记录的三个 integration HEAD/tree 有效。
- 不创建 worktree、不提交、不 push、不操作 stable。

