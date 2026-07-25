# P5-F300 Linked exception instruction facts

状态：Implemented checkpoint。结果见
`P5-F300-linked-exception-sites-result.md`。

## 直接父节点与权威链

- 阻塞结果：`P5-F299-runtime-local-exception-carrier-result.md`
- 共享fixture前置：`P5-F298-service-error-type-index-result.md`
- artifact source/lowering检查点：
  `P5-F286-open-error-language-source-lowering-result.md`
- linked type检查点：
  `P5-F297-applied-nominal-linked-consumer-result.md`

上述父链继续引用唯一权威设计。启动时只读本任务；需要依据时沿父链向上读取。

## DAG位置与依赖

- 节点：F299前置的linked exception instruction facts检查点。
- 语义依赖与F298共享fixture前置均已满足；从包含F298集成提交
  `3ac1dd8264d3118586f088bad155e5615d50653b`的新checkpoint启动。
- 完成后解除重新派发F299。
- 当前是实现检查点，不是稳定候选。

## 唯一production范围

- `runtime/linked-program/src/linked.rs`
- `runtime/linker/src/linker/file_conversion.rs`
- `runtime/linker/src/assembly_execution/code_linker.rs`

仅为required字段迁移与聚焦测试允许修改`runtime/linked-program/**`、
`runtime/linker/**`内直接受影响的tests/fixtures。禁止修改loader、runtime model/eval/boundary/
request/host/transport、artifact/compiler、router/std、生态仓库或权威文档。

## 完成标准

### 1. Linked IR严格保留facts

- `LinkedStmtIr::Throw`与`LinkedExprIr::Throw`持有required
  `InstructionSourceSite`；
- linked `CallIr`持有required `InstructionSourceSite`；
- `LinkedExprIr::Catch.catch_type`是required `LinkedTypeRef`，删除Option、serde default与
  catch-all表示；
- 不增加compatibility default、legacy reader、display/shape推断或linked层synthetic site；
- strict反序列化拒绝缺少上述required字段的linked输入。

### 2. 转换与链接

- file conversion逐字段复制artifact throw/call的exact site；
- file conversion把artifact required catch type直接转换为linked required catch type；
- assembly code linker无条件链接catch type，不保留optional分支；
- site不参与type linking、不改变其Source/Synthetic内容，也不重新编号或重写span；
- F297的AppliedNominal linked/type-plan行为保持不变。

### 3. Consumer边界

- 只迁移本任务crate内因required字段而失败的fixture/literal，并给它们显式真实或
  合法Synthetic site；
- 不替runtime eval/host/driver构造site；这些旧consumer允许在其owner任务前暂时编译失败，
  但必须记录精确首错；
- 不创建request stack、Exception、service envelope或InternalError。

## 最小测试与验证owner

至少覆盖：

- source throw statement、throw expression、local/package/service/native call的site逐值保留；
- Synthetic site逐值保留；
- required catch type保留AppliedNominal；
- linked JSON缺throw site、call site或catch type全部拒绝；
- code linker对required catch进行exact type linking；
- 反搜不存在linked optional catch与conversion `.as_ref().map`降级。

唯一owner：

```bash
cargo test -p skiff-runtime-linked-program --lib -- --list
cargo test -p skiff-runtime-linked-program --lib --no-fail-fast
cargo test -p skiff-runtime-linker --lib -- --list
cargo test -p skiff-runtime-linker --lib --no-fail-fast
git diff --check
```

先确认selector非零。若eval/host等旧consumer只在workspace入口遮挡，不越界修复；记录精确首错。
不运行workspace、runtime-eval、生态、stable、live或chat smoke。

## 风险与交付

- 风险：中；与F299共同进入`A5-runtime-channel`独立验收。
- worktree：`/Users/geek/workspace/skiff-p5-f300-linked-exception-sites`
- branch：`codex/p5-f300-linked-exception-sites`
- 从F298已集成的明确checkpoint创建；不push、不操作stable。
- 启动到第一次production修改不超过5分钟；不可执行时返回
  `TASK_NOT_EXECUTABLE`、精确缺口与最小前置。
- 提交后返回commit、字段迁移、反向搜索、自验收矩阵与所有下游遮挡；不承接F299。
