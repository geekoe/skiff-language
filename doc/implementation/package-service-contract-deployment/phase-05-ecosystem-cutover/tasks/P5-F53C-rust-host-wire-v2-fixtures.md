# P5-F53C：Rust Host/Wire v2 Fixtures

只改D53列出的host non-loader普通正例fixture与transport tests中的两个positive SPI值；明确保留
runtime_config/register_mapper及legacy shape/protocolVersion的v1负例。运行host/transport聚焦测试、
check/rustfmt/diff与允许清单反搜，提交单一commit；禁止完整gate/I02/R05。
