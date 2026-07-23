# P5-F52C：Runtime Register Protocol Version Removal

DAG节点F52C，依赖用户决定删除`runtime.register.protocolVersion`及权威设计更新。独立worktree，唯一
production写入为`runtime/transport` register wire model、`runtime/host` register mapper及对应Rust测试。

从register envelope/model/serialization彻底删除`protocol_version`；host mapper只原样携带canonical v2
ServiceProtocolIdentity，不再从其前缀派生runtime版本。保留frame
`schemaVersion=skiff-runtime-frame-v1`作为唯一transport版本。旧字段输入/shape必须fail closed或因
additionalProperties被Router拒绝，不得dual-read/ignored compatibility。

运行transport/host register聚焦测试、loader扩大suite、check/rustfmt/diff与反搜，提交单一commit。禁止修改
Router文件、I02/R05/instance/stable/full gate/push/merge。
