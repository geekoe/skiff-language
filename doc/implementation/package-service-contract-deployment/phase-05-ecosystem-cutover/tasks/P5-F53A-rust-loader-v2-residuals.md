# P5-F53A：Rust Loader v2 Residuals

只改`runtime/loader/src/lib.rs`与`runtime/host/src/loader/program_loader/tests.rs`：删除本地死v1 SPI owner，
loader正例fallback改用artifact identity canonical v2。不得改legacy reject负例或frame schema。
运行runtime-loader与host loader命名测试、check/rustfmt/diff，提交单一commit；禁止完整gate/I02/R05。
