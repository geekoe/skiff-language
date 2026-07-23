# P5-I55C：Terminal Consumer Reacceptance

依赖F55D/E合流到commit `518fe4a0ac03e72c9be5a4c5ee4ac57aa4accab2`。I55AB loader/linker PASS继续有效。
全新只读owner各运行一次：

```bash
cargo check --locked -p skiff-runtime-loader -p skiff-runtime-linker -p skiff-runtime-host -p runtime
cargo test --locked -p skiff-runtime-host runtime_assembly_request
cargo test --locked -p skiff-runtime-activation
git diff --check
```

静态要求`LinkedImageActivationFacts|LinkedProgramImageCache`零命中且canonical链保留。禁止编辑/gate。
PASS解锁F55C。
