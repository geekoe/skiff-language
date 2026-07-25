# P5-F302 Applied nominal compiler combined integration probe

状态：FAIL。结果见
`P5-F302-applied-nominal-compiler-combined-probe-result.md`。

## 输入与父节点

- language producer：`P5-F296-applied-nominal-compiler-consumer-result.md`
- package/public consumer：
  `P5-F301-applied-nominal-package-public-consumer-result.md`
- 解除原runtime编译遮挡：
  `P5-F300-linked-exception-sites-result.md`

精确候选：integration commit `cadb1283`及其compiler production tree。

## 角色与边界

这是只读combined integration probe，不是独立验收、完整gate或开发任务。不得修改/提交文件，不得
操作stable、live或生态仓库。目标是在F296与F301合流后确认真实compiler integration入口已穿过共同
编译/类型接线，并提前发现直接失败路径。

若命令失败，只返回完整首错、受遮挡范围与mechanical/implementation/owner/design分类；不得顺手修复。

## 唯一证据命令

在`/Users/geek/workspace/skiff-phase-05-integration`执行：

```bash
cargo test -p skiff-compiler --test file_ir_execution_type_representation -- --list
cargo test -p skiff-compiler --test file_ir_execution_type_representation --no-fail-fast
cargo test -p skiff-compiler --test package_imports -- --list
cargo test -p skiff-compiler --test package_imports --no-fail-fast
cargo test -p skiff-compiler --test test_artifact_identity -- --list
cargo test -p skiff-compiler --test test_artifact_identity --no-fail-fast
git diff --check
git status --short
```

每个selector必须非零。PASS后解除`A2-language`独立验收，并允许F269基于该精确compiler checkpoint
重新运行生态消费任务。
