# P5-F52D：Router Register Protocol Version Removal Result

结论：COMPLETE，integration commit `650bcb6`。Router register schema/envelope/registry/snapshot/introspection删除
`protocolVersion`，SPI只接受v2；旧字段与v1 fail closed。protocol 43/43、type-check与diff PASS。扩大registry
suite被遗留v1 manifest fixture阻塞。
