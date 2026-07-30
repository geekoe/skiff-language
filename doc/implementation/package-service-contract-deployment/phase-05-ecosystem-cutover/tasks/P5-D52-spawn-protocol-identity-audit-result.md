# P5-D52：Spawn Protocol Identity Audit Result

结论：COMPLETE。canonical ServiceProtocolIdentity为
`skiff-service-protocol-v2:sha256:<64hex>`。所有真实SPI admission必须一次修复，禁止producer改写、
dual-prefix、fallback、重算或inference。

无争议production面：Router spawn submit/claim request/claim response item及retained legacy request.start；
Host loader service unit validation。renew/complete/fail不携SPI，不纳入。唯一设计缺口是
`runtime.register.protocolVersion`：当前从SPI前缀推导，但runtime transport已有
`schemaVersion=skiff-runtime-frame-v1`；需用户决定删除冗余字段（建议）或定义独立runtime语义。
