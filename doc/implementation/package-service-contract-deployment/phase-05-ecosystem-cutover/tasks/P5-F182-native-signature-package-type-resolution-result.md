# P5-F182：Native 签名中的 Package 类型解析结果

状态：Completed

## 结果

- `NativeSignatureTypeExpr` 新增结构化 `Package { package_id, public_path }`；
  `Builtin` 只再表示 compiler-owned builtin。
- std native 签名中的 `Duration`、HTTP、file、resource 公开类型已迁移为
  `skiff.run/std` 的精确公开类型引用，没有把 std record 或 alias 加入 builtin registry。
- compiler 在装载已验证 platform Package API 后，分别校验：
  - builtin 必须存在于语言 primitive 或 compiler builtin registry；
  - Package 类型的 Package ID 与 public path 必须精确命中已验证公开 API，且目标必须是
    type/alias，不能是 function 或未知符号。
- runtime native call plan 按精确 Package ID 与 public path 查询 admitted
  `RuntimeTypeExports`，再使用对应 `TypeAddr` 构造名义类型计划；删除了
  `Builtin` 分支对 `std.*` 和 `Duration` 的猜测、短路径回退。
- runtime eval 测试 fixture 也改为安装真实的 std Package 名义类型事实，不再让 Package
  类型在缺 Package 的 synthetic program 中隐式成功。

## 失败关闭覆盖

- 相同短名、不同 Package ID 分别解析到各自的 `TypeAddr`；
- 缺 Package、缺 public path、错误符号种类和链接地址错配均拒绝；
- `std.file.CreateOptions` 放入 `Builtin` 分支会按未知 builtin 拒绝；
- compiler 拒绝错误 Package ID、未知公开路径和指向 native function 的公开路径；
- `Nullable<CreateOptions>`、Array/Map/Nullable/Stream 与类型参数递归行为保持原有结构。

## 验证

- `cargo test -p skiff-artifact-model native_signature -- --nocapture`
  - 6 passed
- `cargo test -p skiff-compiler-source prelude_registry -- --nocapture`
  - 21 passed
- `cargo test -p skiff-runtime-linked-type-plan --lib -- --nocapture`
  - 11 passed
- `cargo test -p skiff-runtime-eval --lib -- --nocapture`
  - 85 passed
- `cargo test -p runtime --lib eval::tests -- --nocapture`
  - 101 passed
- `cargo check --workspace`
  - passed
- `git diff --check`
  - passed

补充：`cargo test -p skiff-compiler-source --lib` 的 228 项全量测试仍有 13 项与本任务无关的
既有 actor/interface/callable-effects 失败；本任务要求的 prelude/native signature 聚焦测试全部通过。
