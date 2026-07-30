# P5-F56C0：Trusted Registry Contract Checkpoint

共享短检查点。定义backend-neutral typed trusted registry trait/DTO/signatures：四对象具体put/read、四种typed
pointer read/CAS、typed history selector/receipt；禁止generic kind+JSON/path/bytes。定义exact native signatures、
`NativeRequiredContext::TrustedRegistry`、capability `skiff.registry.trusted@1`与operation scopes
artifact.read/write、pointer.read/cas、history.read。activation prepare/commit/abort DTO只携typed tuple/ref，
不暴露ACK set或state direct-write。

本节点不实现DB/filesystem persistence、不接Internals、不改Router coordinator行为。运行相关crate
check/roundtrip/fail-closed测试、rustfmt/diff，提交单一commit。PASS解锁F56C1–C4。
