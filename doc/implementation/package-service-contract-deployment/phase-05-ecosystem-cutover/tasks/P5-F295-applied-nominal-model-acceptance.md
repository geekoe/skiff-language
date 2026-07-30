# P5-F295 Applied nominal model acceptance

状态：Ready。

## 输入

- implementation result：
  `P5-F294-applied-nominal-shared-model-result.md`
- implementation task：
  `P5-F294-applied-nominal-shared-model.md`
- exact production candidate：
  `e5c36178b35bbfa55d6ce4042053ebe87e1dd257`

结果与任务继续引用owner审计和唯一权威设计。

## 只读验收

独立检查实际candidate，不预设开发总结正确。必须给出 `PASS` 或 `FAIL`：

1. wire只有唯一`AppliedNominal`表示；base closed，arguments required/non-empty/ordered，无default、
   skip、alias或legacy reader；
2. plain/applied single representation、declaration kind/arity/scope与illegal base严格失败；
3. named-union branch旧map删除，nested applied arguments在所有本任务owner traversal/rebind/admission
   中不丢失；
4. exact external owner/ABI expectation保持，不按display/shape猜；
5. applied PackageSchema只在授权admission fail closed，未偷偷修改public contract owner；
6. version/identity matrix精确；argument/base mutation改变应变identity，保持项与human label不变；
7. production scope只落在任务授权model/identity owner，没有compiler/runtime/compat扩张；
8. 开发测试覆盖真实负例，不存在零selector或只测serde round-trip而未测admission/identity。

独立运行：

```bash
cargo test -p skiff-artifact-model --lib -- --list
cargo test -p skiff-artifact-model --lib --no-fail-fast
cargo test -p skiff-artifact-identity --lib -- --list
cargo test -p skiff-artifact-identity --lib --no-fail-fast
git diff --check e5c36178^ e5c36178
```

可增加最小只读反向搜索/抽查；不得修改文件，不运行workspace/compiler/runtime/stable/live。

## 交付

只新增并提交：

`P5-F295-applied-nominal-model-acceptance-result.md`

记录actual commit/tree、PASS/FAIL、blocking issues、证据与残余consumer handoff。FAIL不得顺手修复。
