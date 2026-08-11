# Rust Runtime Error 到 Skiff 的映射讨论

Status: discussion draft, not canonical contract

Last updated: 2026-08-11

## 1. 背景

这次讨论从一个 clippy 问题开始：完整 linker/vm clippy 被 `runtime/boundary` 的存量
`result_large_err` 阻塞，其中最大原因是 `RuntimeError::Recoverable` 内嵌了较大的
`RecoverableBoundaryError`。

讨论过程中，真正的问题逐渐变成：Rust runtime 错误应该如何映射到 Skiff，特别是
recoverable 相关错误是否应该让 Skiff 可见、可 catch，并在跨 service 场景继续传播。

本文只记录当前讨论，不声称已经是最终设计。

## 2. 当前实现

Rust 没有 checked exception。可失败函数使用 `Result<T, E>`，错误通过返回值向上传播。

当前有多个 `RuntimeError`：

- `skiff-runtime-boundary::RuntimeError`
- `skiff-runtime-eval::RuntimeError`
- `skiff-runtime-native::RuntimeError`
- `skiff-runtime-host::RuntimeError`

它们各自定义在不同 crate 的命名空间中，因此同名不冲突。不同 crate 之间通过 `From`
实现手动转换，并不是因为格式相同，而是因为不同执行层需要保留不同控制语义。

共享的扁平化错误结构已经存在：

```rust
pub struct RuntimeErrorPayload {
    pub code: String,
    pub message: String,
    pub status: Option<u16>,
    pub details: Option<Value>,
}
```

对应的 trait 是 `WirePayload`，它让各 crate 的 typed error 能产生同一份
`RuntimeErrorPayload`，并提供可选的 catch projection。

Skiff 侧的异常模型是 `RequestException` / `UserException`。只有带 `CatchIdentity`
的错误才能被 Skiff 的 `catch` 捕获。Rust 内部错误需要先投影成 Skiff value，再包成
`UserException`。

## 3. Recoverable 当前路径

`RecoverableBoundaryError` 定义在：

```text
runtime/boundary/src/error.rs
```

它内部有 `code`、`message`、`context`、`expected`、`detail`。`code` 本身已经承担分类，
例如：

- `UnsupportedEncode`
- `UnsupportedDecode`
- `StateInvalid`
- `ExpectedTypeMismatch`
- `CodeIdentityMissing`
- `ArtifactUnavailable`
- `NativeMissingAdapter`
- `InterfaceConformanceMissing`

`runtime/boundary` 是一个 crate，包名是 `skiff-runtime-boundary`。但 recoverable 不只属于
这一个 crate：

- `runtime/model` 定义 recoverable DTO 和 expected type plan。
- `runtime/boundary` 实现 recoverable codec，并抛出 `RecoverableBoundaryError`。
- `runtime/eval` 使用 recoverable behavior hooks、task dispatch payload。
- `runtime/host` 负责 request、service call、telemetry 的最终边界。

当前 recoverable 错误在 eval 的 `ordinary_catch_projection()` 中返回 `None`，所以 Skiff
的 `catch` 不能直接捕获它。它最终通常被 `export_provider_failure` 固化为
`std.service.InternalError`，或者在 request boundary 变成 `response.error`。

## 4. 讨论中形成的共识

### 4.1 Rust 内部错误模型

建议不是“一个 crate 一种具体错误类型”，而是“一个 crate 一个 `RuntimeError` enum，概念对应
variant”。

例如：

```rust
pub enum RuntimeError {
    DecodeTarget { target: String, message: String },
    Recoverable { ... },
    HttpError { message: String, detail: Option<Value> },
    ...
}
```

不要为每个错误概念都建一个独立 struct，否则边界之间的 `From` 映射会重复膨胀。

### 4.2 RecoverableBoundaryError 还需要存在吗

作为 Skiff 类型，不需要。

作为 Rust 内部类型，仍建议存在，至少需要保留 recoverable 错误所需的字段：

- code
- message
- nodePath
- boundaryKind
- detail

它可以继续是共享 struct，也可以内联成 `RuntimeError::Recoverable { ... }`。关键不是名字，
而是内部错误在到达 Skiff 前保留足够的诊断字段。

`Boundary` 这个名字表示它来自 runtime boundary codec 层，不是必须保留的字眼。如果觉得容易
混淆，可以改成 `RecoverableCodecError` 或 `RecoverableFailure`。

### 4.3 recoverable crate 的错误不都是 RecoverableBoundaryError

正确。

`runtime/boundary` 这个 crate 还处理普通 binary、JSON、HTTP、file、DB codec。它产出的错误
包括 `DecodeTarget`、`BytesDecode`、`DbDecode`、`FileError`、`HttpError` 等。

只有 `runtime/boundary/src/recoverable.rs` 这个 recoverable 模块产出的错误，才应该收敛成
`RecoverableBoundaryError`。

JSON 错误需要区分：

- 普通 `std.json.decode` 失败，应该是 `DecodeTarget("std.json.decode")`，最终变成
  `std.json.DecodeError`。
- recoverable envelope 内部恢复 JSON 数据失败，应该包成 `RecoverableBoundaryError`，例如
  `ExpectedTypeMismatch` 或 `StateInvalid`，并把原始错误放进 detail。

### 4.4 到 Skiff 的映射应该扁平化

所有 Rust 内部错误在 Skiff 边界都可以收敛成几个稳定字段：

```text
code
message
traceId
errorId
details
```

这符合现有 `RuntimeErrorPayload` 的方向。Skiff 不需要看到 Rust enum、expected type plan 或
内部 stack。

一个候选 Skiff 类型：

```skiff
type RuntimeError {
  code: string
  message: string
  traceId: string
  errorId: string
  details: Json?
}
```

如果采用这个单一类型，Skiff 只 catch `RuntimeError`，再根据 `code` 判断具体错误。这能极大
减少错误映射代码，但代价是失去按类型 catch 的精确性。

如果保留 `catch<std.json.DecodeError>` 这类精确 catch，就需要保留具体公开错误类型，同时用
扁平字段作为通用 fallback。

## 5. 候选映射规则

```text
RecoverableBoundaryErrorCode::*  -> std.service.RecoverableError
DecodeTarget("std.json.decode")  -> std.json.DecodeError
BytesDecode                      -> std.bytes.DecodeError
DbDecode                         -> std.db.DecodeError
FileError                        -> std.file.FileError
HttpError                        -> std.http.HttpError
ExecutionBudgetExceeded          -> TimeoutError
Protocol                         -> std.service.ProtocolError
ProviderUnavailable              -> std.service.ProviderUnavailableError
其它                              -> std.service.InternalError
```

建议把这张表放在一个投影器里，而不是继续在 eval、native、host、request 各维护一套 match。

一个可能的 trait 形态：

```rust
pub trait SkiffErrorProjection: WirePayload {
    fn skiff_error_kind(&self) -> SkiffErrorKind;
    fn project_user_exception(
        &self,
        context: ProjectionContext<'_>,
    ) -> Result<Option<RequestException>>;
}
```

这样各 crate 仍然保留自己的 `RuntimeError` enum，但“变成哪个 Skiff 错误、怎么 materialize、
怎么跨 service”只在一个地方维护。

## 6. Recoverable 传给 Skiff 的候选实现

如果决定让 recoverable 错误在 Skiff 内可 catch，建议按下面顺序改。

### 6.1 新增公开错误类型

在 `std/service.skiff` 增加：

```skiff
type RecoverableError {
  code: string
  message: string
  nodePath: string
  boundaryKind: string
  traceId: string
  errorId: string
}
```

在 `std/api.yml` 的 `service` 段导出：

```yaml
RecoverableError: service.RecoverableError
```

同时更新 std surface checker 和 compiler 的 public symbol 断言。

### 6.2 在 eval 中物化成 UserException

在 `runtime/eval/src/exceptions.rs` 增加 helper，类似现有的
`request_exception_for_resource_error`。

它负责：

- 解析 `std.service.RecoverableError` 的公开类型 identity 和 schema plan。
- 从 `RecoverableBoundaryError` 投影稳定字段。
- 构造带 catch identity 的 Skiff value。
- 返回 `RequestException::local(...)`。

然后在 `promote_call_site_error` 中，遇到 `RuntimeError::Recoverable` 时调用这个 helper，
包成 `UserException`。

### 6.3 未捕获时跨 service 传播

未捕获的 `UserException` 会走现有 `export_local_exception`。只要
`std.service.RecoverableError` 是公开且 schema-closed，它就会变成
`ServiceErrorEnvelope::PublicTypedError`。

caller service 通过现有 `import_caller_failure` 恢复成同名 Skiff value，因此可以：

```skiff
catch<std.service.RecoverableError>
```

如果 caller 没有链接该类型，envelope 仍可作为不可匹配的异常因果继续转发。

### 6.4 fallback 保留

如果 `std.service.RecoverableError` 解析失败、schema 不合法或编码失败，仍然走
`fixed_internal()`，降级为 `std.service.InternalError`。

## 7. 需要另一个人确认的问题

1. Recoverable 错误是否允许在同一个 request 内被 Skiff `catch`？这会改变当前“平台错误不可
   catch”的语义，需要同步更新 runtime 文档。
2. 是否接受单一公开 `RuntimeError { code, message, traceId, errorId, details }`？还是保留
   `std.json.DecodeError` 等具体类型？
3. `std.service.RecoverableError` 应该暴露哪些字段？是否要包含 `nodePath`、
   `boundaryKind`，还是只保留 `code`、`message`、`traceId`、`errorId`？
4. `RecoverableBoundaryError` 应该继续作为共享 struct，还是内联成
   `RuntimeError::Recoverable { ... }`？
5. 中央投影器应该放在 `runtime/request-contract`、`runtime/eval`，还是新建 crate？
6. ingress 阶段发生在 Skiff handler 之前的 decode 错误，是否仍保持不可 catch？
7. 这个设计是否要和 `result_large_err` 的 refactor 一起做，还是分开？

## 8. 相关代码位置

- `runtime/boundary/src/error.rs`: `RecoverableBoundaryError` 和 boundary
  `RuntimeError`。
- `runtime/eval/src/error.rs`: eval `RuntimeError`、`ordinary_payload()`、
  `ordinary_catch_projection()`。
- `runtime/eval/src/eval_context.rs`: `promote_call_site_error`，Skiff 调用点错误提升。
- `runtime/eval/src/exceptions.rs`: `request_exception_for_catch` 和
  `request_exception_for_resource_error`。
- `runtime/eval/src/assembly_execution/service_error_channel.rs`: provider 错误导出和 caller
  错误导入。
- `runtime/request-contract/src/error.rs`: `RuntimeErrorPayload` 和 `WirePayload`。
- `std/service.skiff`: 现有 `ProviderUnavailableError`、`ProtocolError`、
  `InternalError`。
- `std/api.yml`: std public surface。
