# P5-F54B：Remove Legacy Service Register Test

只改`runtime/host/src/loader/runtime_config.rs`测试：保留artifact loader revision/build断言，删除经test-only
`RuntimeConfig.services`伪造旧service register frame的段落与孤立helper/import，并重命名测试。
运行该测试及既有exact assembly reconnect/full-chain registration测试、rustfmt/diff，提交单一commit。
禁止production改动、compat service registration、I02/R05/full gate。
