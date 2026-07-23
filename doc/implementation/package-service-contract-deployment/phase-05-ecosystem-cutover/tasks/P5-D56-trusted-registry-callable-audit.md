# P5-D56：Trusted Registry Callable Audit

依赖T08 TASK_NOT_EXECUTABLE。两个全新只读分片并行：

- D56A：审计现有Skiff package/native/Host callable如何安全暴露trusted Rust能力，给四对象immutable write/read、
  typed pointer CAS/history及activation prepare/commit/abort的最小production owner/API/权限边界。
- D56B：对照权威设计/T01/T08，判定Platform DB、CanonicalArtifactStore、release pointer与Router coordinator的
  source-of-truth关系是否已明确；若明确给漏实现前置DAG，若不明确给最小用户选择。

禁止编辑、提交、stable/gate。不得建议raw JSON、path复制、ambient filesystem、dual store或Internals重实现identity。
