# P5-F56C1：Platform DB Registry Adapter

基于F56C0实现production Platform DB backend，四类immutable records、typed pointer current/history与activation
state/audit为唯一durable SOT。复用Rust typed validation/identity/CAS，不接受generic JSON/path；CAS成功与history
原子/可恢复提交，activation state+audit同事务。文件CanonicalArtifactStore不得dual-write。

写入deployment/backend及必要DB adapter测试，不接Host native或Internals。覆盖immutable conflict、stale CAS、
history sequence、transaction rollback/recovery。check/test/rustfmt/diff后单commit；禁止stable/full gate。
