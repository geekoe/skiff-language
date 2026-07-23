# P5-F52C：Runtime Register Protocol Version Removal Result

结论：COMPLETE，integration commit `43e4d89`。transport/host register model与serialization彻底删除
`protocolVersion`，旧字段deny-unknown fail closed；host mapper只校验并原样携带canonical v2 SPI。transport
2/2、mapper 7/7、checks/rustfmt/diff PASS。扩大loader suite的2项失败归因到`runtime/loader`残留v1。
