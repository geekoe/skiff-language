# P5-P26S：Source Diagnostic Batch

使用全新只读执行Agent，锚定D26 docs checkpoint；不修改提交，不运行combined/I16/H18/full/Host/stable。以一个
exclusive nonce/marker/dev+ino task root拥有专用cold Cargo target，绝不触碰Gate shared target或stable。开始前要求可用
空间至少`33,207,861,248 B`，否则创建/build前`PREFLIGHT BLOCKED`。

真实依赖顺序，任一步FAIL立即停止且不重试：

1. cold empty callback：从非repo cwd调用`runInIsolatedTestRuntime`，`skiffRoot`为integration absolute root，base env只把
   `CARGO_TARGET_DIR`设为owned cold target；callback恰好一次且不启动source runner。PASS须generation-0 exact ready，随后
   supervisor/instance/process group/ports/lease/inner temp全ABSENT。
2. 复用P20A official std exact PASS，不重跑；只执行一次
   `fresh_helper_mutation_then_detached_service_call_projects_and_assembles` exact Rust test，必须1/0/0。
3. cold与helper均PASS后，在同一owned target再次`runInIsolatedTestRuntime`；callback只运行现有
   `skiffSourceTestRunnerCargoArgs`对`std`，explicit `--bin skiff-test-runner`、deny-skips/require-tests，捕获原始
   stdout/stderr。PASS须exact 11个PASS、唯一`test result: ok. 11 passed; 0 failed`，Host preparer/consumer为0。

失败回报阶段、首个原始diagnostic与owner分类；禁止继续后步。finally仅在nonce/marker/dev+ino复验后清理task target，
foreign replacement保留；记录所有PID/port/path absence。不得创建production `--std-only`或新的Host harness。
