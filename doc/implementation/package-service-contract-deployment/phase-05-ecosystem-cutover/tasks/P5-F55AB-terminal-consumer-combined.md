# P5-F55AB：Terminal Consumer Combined

依赖F55A/B合流到commit `905c1a8c1e687ac2b0fa9e5acd62fdff468354c4`。全新只读owner各运行一次：

```bash
cargo check --locked -p skiff-runtime-loader -p skiff-runtime-linker -p skiff-runtime-host -p runtime
cargo test --locked -p skiff-runtime-loader runtime_assembly
cargo test --locked -p skiff-runtime-linker assembly
cargo test --locked -p skiff-runtime-host runtime_assembly_request
git diff --check
```

静态确认canonical RuntimeAssembly/AssemblyExecutionImage/shared HTTP/request heap保留，旧graph/cache/program/
service route/driver facade为零。禁止编辑/I02/R05/full gate。PASS解锁F55C linked-program facade removal。
