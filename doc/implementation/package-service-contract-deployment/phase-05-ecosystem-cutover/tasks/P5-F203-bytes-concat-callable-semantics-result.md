# P5-F203：bytes.concat callable semantics 结果

状态：Completed

## 结果

`core.bytes.concat` 现在进入编译器拥有的精确 native callable semantics
registry。它只匹配既有 canonical binding key 与签名
`(Array<bytes>) -> bytes`，返回 provenance 为 `Fresh`，并明确声明：

- 不写 caller 可达值；
- 不返回或抛出 caller alias；
- 不逃逸 caller 值；
- 不要求同一 heap identity；
- 不调用未知目标；
- 不挂起。

没有修改 Runtime handler，也没有添加 OpenAI 或其他 Package 的特例。

## 失败关闭

新增测试覆盖：

- `std.bytes.concat`、`bytes.concat`、带后缀 binding key 和其他 concat
  lookalike 不继承精确语义；
- 缺失参数、多余参数、`Array<string>` 参数和错误返回类型均在 canonical
  native signature 校验中失败；
- 正确调用精确 lower 到 `core.bytes.concat`。

## OpenAI 链路

源码 callable-effects 测试使用官方 OpenAI multipart 的核心形状：
构造 `Array<bytes>` chunks，加入 multipart boundary、part body 和尾部，
最终调用 `bytes.concat(chunks)`。`multipartBody` 完整分析为无 effects，
返回 provenance 为 `Fresh`，且 resolved call target 是精确的
`core.bytes.concat`；不再产生 `UnknownCallTarget` 或
`requiresSameHeapIdentity`。

真实官方 OpenAI `skiff test` 尝试三次，均在执行任何 Package
编译、准入或测试前被独立的隔离环境启动故障阻断：

```text
[skiff-instance] started mongo
[skiff-instance] supervisor failure: router exited after start with 1
error: isolated runtime supervisor exited while initializing MongoDB
```

三次使用不同的动态隔离目录和进程，故障阶段一致；隔离测试设施按约定自动删除临时目录。
该故障没有重新出现 `UnknownCallTarget`，但由于 Router 未启动，不能把它记为 OpenAI
测试通过。后续应在隔离 Router 启动恢复后直接复跑。

## 验证

- `cargo test -p skiff-artifact-model bytes_concat_semantics_match_exact_signature`
- `cargo test -p skiff-compiler-source bytes_concat_openai_multipart_shape_uses_exact_native_semantics`
- `cargo test -p skiff-compiler bytes_concat_lowers_to_exact_native_binding_and_rejects_malformed_calls`
- `cargo check --workspace`
- `git diff --check`

上述静态、聚焦与 workspace 验证均通过。现存编译 warning 为基线 warning。
