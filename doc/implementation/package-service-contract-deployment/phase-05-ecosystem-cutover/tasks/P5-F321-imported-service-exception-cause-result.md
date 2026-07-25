# P5-F321 Imported service exception cause result

状态：PASS。

实现提交：`fb2737ede6b0b76d63593a2186c8be9f6a012f08`。

## 结果

- request exception的remote cause已收敛为必选`OpaqueServiceError`加可选caller-local
  `RuntimeValueCarrier`；没有保留旧的第二变体。
- linked public/platform/Internal inbound可以exact catch，同时保留原始fixed bytes。
- unlinked public inbound的local value为`None`，catch必定miss，但fixed accessor仍返回同一原始envelope。
- `map_local_value`可移动local或linked imported carrier；对`None`不调用closure、不凭空materialize，并且
  从不改写raw bytes。
- local cause没有fixed accessor；local rethrow的source、stack、correlation及`Local` cause保持不变。
- imported correlation由strict fixed envelope派生；malformed envelope仍由原有decoder拒绝。

Rust变体保留`OpaqueService`名称，但字段和API已经是唯一imported语义，不存在legacy opaque-only变体。

## 验证

- runtime-model test list：84，非零。
- runtime-model full：84/84 PASS。
- crate `rustfmt --check`与`git diff --check`：PASS。
- 唯一production修改是`runtime/model/src/service_error.rs`。

