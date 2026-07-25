# P5-F316 Representation wrap linked consumer结果

状态：Completed。

任务提交：`c9e470d7697526fa7b9d0541044030e6ae621dec`。

集成提交：`74049d55a3e2434940494cf83dce9fcb02c5ae8f`。

## 结果

- 唯一strict `LinkedExprIr::RepresentationWrap { value, type_ref }`；
- conversion精确保留child、plain/applied target、PackageSymbol owner/ABI与nested ordered arguments；
- code linker递归解析base/arguments并保留AppliedNominal wrapper；
- exact kind admission只接受Representation；
- record/union/alias/interface、wrong arity/owner/ABI、PackageSchema、DbObjectSymbol与direct/nested
  TypeParam全部fail closed；
- type plan按exact owner+arguments区分local/external与`R<string>`/`R<number>`，未实现eval carrier。

## 验证

- linked-program：PASS，34/34；
- linker：PASS，45/45；
- linked-type-plan：PASS，17/17；
- 聚焦wire 3/3、linker 5/5、type-plan 1/1；
- fmt与`git diff --check`：PASS。

本结果解除F318 eval consumer。

