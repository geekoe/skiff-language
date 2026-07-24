# P5-F182：Native 签名中的 Package 类型解析

状态：Ready

## 直接父任务

- `P5-F181-compiler-owned-builtin-prelude-result.md`

## 问题

`NativeSignatureTypeExpr::Builtin` 当前同时承载两种不同语义：

- 语言内置类型，例如 `string`、`bytes`、`Array`、`Map`；
- std Package 源码声明的普通公开类型，例如 `std.file.CreateOptions`、
  `std.http.HttpRequest`、`std.resource.ResourceInfo`。

F181 将真正的 builtin 收归 compiler 后，runtime linked type plan 会把第二类也拿到 builtin registry
查找，导致 `native signature references unknown builtin type std.file.CreateOptions`。不得通过把普通
std record 加入 compiler builtin registry 来绕过。

## 目标

在 native signature 类型表达式中明确区分语言 builtin 与 Package 公开类型，并让 compiler 与
runtime 使用各自已有的 canonical Package 类型事实解析后者。

## 范围

- `artifact-model` native signature 类型表达式及定义；
- `compiler/source` prelude/native signature 装载与类型解析；
- `runtime/linked-type-plan` native call plan；
- 必要的 linker/linked-program handoff 和聚焦测试。

不得修改 std 源码类型声明，不得把 `CreateOptions`、`HttpRequest`、`ResourceInfo` 等普通 record
加入 compiler builtin registry。

## 必须实现

- 将 `NativeSignatureTypeExpr::Builtin` 限定为真正的 compiler-owned builtin；
- 为 Package 公开类型建立专用、名义化的表达式，至少携带精确 Package ID 与 public path，不能只靠
  任意字符串或 source 文件布局猜测；
- 把现有 ImmutableFile、CreateOptions、FileInfo、HTTP 类型、ResourceInfo 等签名迁移到 Package
  类型表达式；
- compiler 解析 native signature 时：
  - builtin 只从 compiler builtin registry 解析；
  - Package 类型只从已加载、已验证的 Package 公开声明解析；
- runtime linked type plan 必须把 Package 类型解析到 admitted linked nominal type，不得伪造 builtin、
  record descriptor或本地索引；
- 缺 Package、缺 public path、Package ID 错配、类型种类错配和伪装成 builtin 全部失败关闭；
- ordinary builtin、类型参数、Array/Map/Nullable/Stream 递归组合行为保持不变。

## 验证

- `std.file.CreateOptions` nullable 参数的真实 native call plan 成功；
- ImmutableFile、FileInfo、HTTP 类型、ResourceInfo 各有至少一个真实正例；
- 相同短名但不同 Package 不会串用；
- Package 类型写入 Builtin 分支必须拒绝；
- 未知 builtin、未知 Package/public path、错误类型种类继续拒绝；
- compiler prelude/native signature 聚焦测试；
- runtime linked-type-plan 聚焦测试；
- 原先失败的 `runtime --lib eval::tests` 两项通过；
- `cargo check --workspace`；
- `git diff --check`；
- 独立提交并写 `P5-F182-native-signature-package-type-resolution-result.md`。

