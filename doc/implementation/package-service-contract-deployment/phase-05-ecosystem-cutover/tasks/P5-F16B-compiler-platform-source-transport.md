# P5-F16B：Compiler Platform Source Transport

## 输入与DAG

- 权威设计：`doc/architecture/package-service-contract-deployment.md` §3、§9、§10、§14；D18与F16A合同。
- 输入：F16A exact checkpoint；从其合流commit建立
  `/Users/geek/workspace/skiff-p5-f16b-compiler-platform-transport`、分支
  `codex/p5-f16b-compiler-platform-transport`。与F16C并行，禁止修改其owner。
- 高风险production authoring consumer；一个clean commit，不merge/push、不改stable。完成后与F16C共同解除I16。
- 使用新的开发Agent，不复用F16A、D18 auditor或文档reviewer。
- 五分钟内开始实际修改；若F16A API不能在本owner内直接消费，立即报`TASK_NOT_EXECUTABLE`，不得回改shared owner。
- 证据只对F16A exact API、compiler bin/authoring JS caller、platform source内容、Cargo.lock与本任务commit不变时有效。

## 写入owner与完成态

owner限compiler binary transport、`scripts/lib/package-service-authoring.mjs`及对应compiler/Node参数测试。消费F16A
唯一context，不创建第二platform-root resolver；不得修改F16A-owned `compiler/driver/authoring.rs`或pipeline。

- internal compiler binary的四对象authoring action严格接收一次`--platform-source-root <absolute-root>`并构造F16A
  context；missing、重复、relative、不可canonicalize或invalid layout均在任何package读取前失败。无源码hidden
  `__ecosystem-store` action保留既有argv且不得构造platform context，这是唯一例外。
- `runCompilerAuthoring`从已有absolute `skiffRoot`给每个compiler invocation传同一内部参数；改变cwd不改变值。
- 用户级`skiff package|contract|deployment|assembly`命令的公开参数/help不增加platform trust选项；既有caller只传
  `skiffRoot`，不从cwd/env推导。
- 删除所有compiler authoring production caller中的隐式platform路径；共享`CARGO_TARGET_DIR`继续支持。

不改F16A shared context/prelude、test-runner、source-suite、`scripts/skiff.mjs` test入口、Router/Runtime、manifest/lock。
直接触碰大文件需extra-review，不顺手重构无关authoring逻辑。

## 唯一聚焦验证

```bash
cargo test --locked -p skiff-compiler --bin skiff-compiler
cargo check --locked -p skiff-compiler --bin skiff-compiler
node --test scripts/tests/package-service-authoring.test.mjs scripts/tests/package-service-store.test.mjs
cargo fmt --all -- --check
git diff --check
```

参数测试必须断言compiler argv携带exact absolute root、cwd变化不变、missing/duplicate/relative fail closed，且公开
user CLI没有新增开关。不得运行source-suite、Host或完整verify。回报commit/tree/lock blob、反向搜索与
extra-review自验收矩阵。
