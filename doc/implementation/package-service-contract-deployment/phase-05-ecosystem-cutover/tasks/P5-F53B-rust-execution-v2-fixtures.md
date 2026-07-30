# P5-F53B：Rust Execution v2 Fixtures

只改D53列出的`runtime/eval`、`runtime/request`、`runtime/driver`普通正例fixture，把实际SPI值切到artifact
identity canonical v2。不得改production逻辑、legacy负例或runtime frame schema。运行各crate命名测试、
check/rustfmt/diff，提交单一commit；禁止完整gate/I02/R05。
