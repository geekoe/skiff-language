# P5-F140：Service Stream Runtime Full-chain Probes

状态：Ready

## 权威设计与 DAG

- 权威设计：`doc/architecture/package-service-contract-deployment.md`。
- 稳定语言语义参考：`doc/reference/runtime.md` 的普通 service `Stream<T>` 生命周期。
- 节点：C1 Runtime 证据 D；前置 P5-D82、P5-F138，可与 P5-F139 并行。
- 完成后解除：AIHub → Agine consumer 重验的 Runtime 证据。

## 写入范围

- `runtime/eval/src/assembly_execution/` 的测试与必要测试 helper。
- 仅在测试暴露既定 Runtime lane 的直接实现缺陷时，允许同目录最小修复；不得改变 wire、公共生命周期或 compiler。

## 完成标准

- 从 synthetic resolved service binding 经过真实 in-process dispatcher 执行 server stream，证明两个 item 顺序、end、
  detached item materialization 与 generic item substitution。
- 覆盖 provider error、consumer break/cancel、caller/request cancel，证明 lease/task 清理且只终止目标 stream。
- 不使用 HTTP adapter、手工直接调用最终 materializer 或绕过 assembly resolve 的假链路。

## 验证

- Runtime eval assembly/stream 聚焦测试、目标文件格式与 `git diff --check`；不运行完整 gate。
- 若测试暴露公共语义缺口或需跨 owner 修复，返回 `TASK_NOT_EXECUTABLE`。

## Worktree

- `/Users/geek/workspace/skiff-p5-f140`
- branch `codex/p5-f140-service-stream-runtime-probes`
- 一次性开发会话；提交、不 push、不操作 stable。

