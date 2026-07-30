# P5-F181 compiler-owned builtin prelude result

## 结果

编译器现在通过一张自有注册表定义 builtin 类型的规范符号、类型参数个数和类别。注册表覆盖当前 prelude 闭包，包括 Actor、bytes、Array、Map、Config、Date、Json、JsonObject、Stream、异常类型闭包和 session capability 类型。

prelude 与官方 std 源码只继续提供函数和方法表面，不再承担 builtin 类型身份声明。`std.websocket` 中有源码声明的公开数据类型仍然是普通 package 类型，没有被错误迁移为 builtin。

## 行为

- bare name 与规范限定名从同一注册表解析。
- 官方平台源码校验、schema-stable 闭包和 prelude 可见类型共享注册表事实。
- builtin 泛型参数个数错误、未知类型和官方源码同名伪造声明均关闭失败。
- builtin 没有被注入 `TypeDecl`，因此不会作为普通 record 或 package schema named record 投影。
- `std/api.yml` 的 actor 表面已与 F178 的四个实际函数同步；Actor 类型身份由 compiler registry 提供。
- 注册表事实进入 prelude identity，相关固定 identity 测试已更新。

## 验证

- `cargo test -p skiff-compiler-core`：39 passed。
- `cargo test -p skiff-compiler-source prelude_registry`：20 passed。
- `cargo test -p skiff-compiler --lib`：16 passed，原有 5 个 authoring 失败全部修复。
- `cargo check --workspace`：通过。
- `rg -n '^native type' prelude std`：无匹配。
- `git diff --check`：通过。
