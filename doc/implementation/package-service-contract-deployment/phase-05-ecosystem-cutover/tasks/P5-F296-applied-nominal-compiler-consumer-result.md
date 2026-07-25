# P5-F296 Applied nominal compiler consumer结果

状态：Implemented checkpoint。

任务提交：`8366c82a6922383c8a205231271368a5c2f68798`。

集成提交：`a933d730124cc2b786927bbe8afc33735e305bd4`。

## 直接任务与权威链

- `P5-F296-applied-nominal-compiler-consumer.md`
- 任务继续引用F295、F286与F293父链。

## 结果

- source按exact declaration owner与arity产生ordered `AppliedNominal`，非法base或参数fail closed；
- core/source/lowering的walk、replace、closure、assignability、conformance递归处理arguments；
- File IR覆盖generic declaration、construct、pattern、throw/catch/rethrow与call type arguments；
- named-union不再产生旧argument map；
- `CatchLeaves`保留structured owner/arguments，覆盖generic representation、named union及branch
  substitution；
- 保持F295冻结的v7/v5 wire与F285/F286/F290语义。

## 验证

- compiler core list/full：PASS，44/44；
- compiler source list/full：PASS，310/310；
- compiler lowering list/full：PASS，47/47；
- 改动Rust文件`rustfmt --check`：PASS；
- `git diff --check`：PASS；
- 反搜`resolved_type_arg_texts`、旧union argument maps与文本替换/recovery helper：production零命中。

两个compiler integration target在枚举前被下游旧consumer遮挡：

- runtime loader仍引用旧`BoundaryErrorContract`与`operation.contract.errors`；
- compiler projection仍使用旧`Union { variants }`并未穷举`AppliedNominal`。

前者进入runtime channel迁移；后者由S2 package/public fail-closed consumer负责。本检查点解除S2、
compiler combined probe与A2-language独立验收。

