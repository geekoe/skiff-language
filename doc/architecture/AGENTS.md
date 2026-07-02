# Skiff Architecture Agent Instructions

本目录保存长期内部架构契约，面向 compiler、runtime、router 和 artifact
维护者。它不定义用户可见语言语义，也不是临时实现计划。

文档分工：

- `../reference/`：用户可见、稳定的语言和 publication 语义。
- `../architecture/`：内部阶段、输入输出、跨系统 contract 和长期边界。
- `../implementation/`：迁移计划、审计记录、阶段性实现方案和问题记录。

维护规则：

- architecture 文档可以写内部 Rust-ish 类型草图，但这些类型不是 public API。
- 不把临时迁移步骤写成长期 contract；迁移步骤留在 `implementation/`。
- 不把用户不需要理解的 compiler/runtime 内部结构写进 `reference/`。
- 如果 implementation 文档和 architecture contract 冲突，以 architecture contract 为准；
  implementation 文档应更新或归档。
