# P5-F139：Service Stream Boundary Projection

状态：Ready

## 权威设计与 DAG

- 权威设计：`doc/architecture/package-service-contract-deployment.md`。
- 稳定语言语义参考：`doc/reference/runtime.md` 的普通 service `Stream<T>` 边界。
- 节点：C1 shared compiler checkpoint A；前置 P5-D82、P5-F137。
- 完成后解除：contract stream caller typing 与真实 lowering fixture。

## 写入范围

- `compiler/projection/src/package_artifact/boundary/`
- `compiler/projection/src/package_artifact/tests/`
- 必要的同 crate 小型 helper；不得修改 artifact schema、Runtime、HTTP boundary 或 caller source typing。

## 完成标准

1. 公开 callable 最外层返回 `Stream<T>` 投影为既有 `ServiceOperationStream::ServerStream`，使用 canonical item type
   与 item value plan；不得先生成 unary return 再适配。
2. Stream 参数、嵌套 Stream、collection 中 Stream、无法 materialize 的 item 保持结构化 unavailable/fail closed。
3. Package public nominal item type closure 与 type identity 保持 canonical。
4. HTTP `HttpResponseStreamEvent` owner 不被复用为 service-call stream。

## 验证

- compiler projection boundary 聚焦测试；至少包含正例、上述关键负例与 deterministic receipt。
- `git diff --check` 和目标文件格式检查；不运行完整 gate。
- 若 item/error/end、cancel 或公共 schema 需要新语义，返回 `TASK_NOT_EXECUTABLE`。

## Worktree

- `/Users/geek/workspace/skiff-p5-f139`
- branch `codex/p5-f139-service-stream-projection`
- 一次性开发会话；提交、不 push、不操作 stable。

