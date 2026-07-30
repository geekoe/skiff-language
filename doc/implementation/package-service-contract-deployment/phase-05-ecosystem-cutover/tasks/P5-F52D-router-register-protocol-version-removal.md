# P5-F52D：Router Register Protocol Version Removal

DAG节点F52D，依赖用户决定与权威设计更新。独立worktree，唯一production写入为Router register protocol、
registry/snapshot/introspection、README及对应TS测试。

删除register schema/envelope/registry snapshot中的`protocolVersion`；register
`serviceProtocolIdentity`只接受canonical v2；frame `schemaVersion=skiff-runtime-frame-v1`继续作为唯一transport
版本。旧`protocolVersion`字段因additionalProperties fail closed，legacy v1 SPI明确拒绝。不得保留optional
field、silent ignore、dual-read或从SPI派生。

运行protocol/registry聚焦测试、Router type-check、diff与反搜，提交单一commit。禁止修改Rust文件、
I02/R05/instance/stable/full gate/push/merge。
