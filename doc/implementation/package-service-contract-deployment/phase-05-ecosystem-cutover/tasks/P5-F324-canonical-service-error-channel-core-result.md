# P5-F324 Canonical service error channel core result

状态：PASS for development scope；等待合流探针与独立R0验收。

实现提交：`8338442277d2d5cb1330dd512f9d0c0d97864e5c`。

## 冻结API

```rust
CanonicalServiceErrorChannel::export_provider_failure(
    actual_error: &RuntimeError,
    context: ServiceErrorExportContext<'_>,
    next_correlation: impl FnOnce() -> Result<ErrorCorrelation>,
) -> Result<OpaqueServiceError>

CanonicalServiceErrorChannel::import_caller_failure(
    error: OpaqueServiceError,
    context: ServiceErrorImportContext<'_>,
) -> Result<UserException>
```

配套共享点：

- `RuntimeError::FixedServiceFailure(OpaqueServiceError)`及安全accessor；
- exact named-union branch lookup与service-error local value materialization helper；
- provider service stack scope/reset API。

## 结果

- imported/fixed cause再次export时最先命中，raw bytes与correlation原样透传。
- public record、representation、named union及dependency-owned error按exact index/schema/selected codec
  导出；owner不改写。
- private/non-nameable/nonclosed/encode failure和普通provider fault只生成一次固定脱敏Internal。
- exact local`std.service.InternalError`进入fixed Internal分支；imported Internal不再包装。
- platform error只由有限enum-keyed registry编解码；ResourceError不进入platform。
- caller exact link得到local carrier+raw envelope；没有exact edge时为合法opaque且catch miss。
- owner/key/id/build/ordinal/payload冲突严格返回Protocol/InvalidArtifact。
- Internal import要求exact std schema link并恢复三字段普通名义值。
- remote import创建新local stack，只加入一个安全RemoteBoundary；provider scope清空继承stack，local rethrow
  保持不变。

## 验证

- core selector：13，非零。
- focused core tests：13/13 PASS。
- `cargo check -p skiff-runtime-eval --lib`：PASS。
- crate `rustfmt --check`与`git diff --check`：PASS。
- production只修改任务授权的五个owner；没有接R1–R3 lane、host/transport/router/stable/live。

production core module约1157行、co-located tests约1435行。F327独立验收必须按workspace代码规范判断它是否
仍是一个清晰的canonical职责，或是否已混合可独立owner而构成blocking结构问题；开发结果不预设该结论。

