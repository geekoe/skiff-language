# Router Rust Migration Archive

Router Rust migration 已完成；当前长期契约以
`doc/architecture/router-rust.md` 为准。本目录只保留迁移期材料，不是第二份 architecture
规范。

- `contracts/`：迁移期冻结的 contract pack。部分 runtime/router corpus 和源码注释仍引用这些
  文件，因此暂时保留。
- `execution/`：batch、leaf、gate、ledger 与迁移计划等执行记录，只用于追溯。

新增 Router 规则应直接更新 `doc/architecture/router-rust.md` 或对应 protocol fixture，不应继续向
本归档追加设计语义。
