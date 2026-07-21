# P5-R06：Callable Effects Transfer Acceptance

未参与F06实现的独立只读Agent。输入为F06 exact clean integration commit/tree、D07/F06合同与聚焦证据；不得
编辑、提交、修复或给F04/R02 verdict。

必验：

- only exact call-position dependency callee绕过standalone address unknown；动态/first-class/unresolved仍fail closed；
- known contract state只来自exact全detached unary descriptor，error/callback/non-detached/missing/ambiguous保持unknown；
- 直接标量参数字段写只产生write effect，nested/index/reference/unknown store仍全量fail closed；
- existing provenance-aware `apply_callee`使helper mutate自身Unavailable、fresh actual consumer Available，不靠symbol
  白名单或fixture特殊分支；
- package/contract dependency facts仍从canonical artifact/index读取，projection/lowering/artifact/runtime无越界修改；
- `extra-review`检查call/expression/statement transfer无重复target resolver、隐藏fallback或职责混杂。

第一行只给`R06 PASS`或`R06 FAIL`。PASS只解锁F04恢复真实consumer isolated正例；FAIL给最小source反例、
facts差异和唯一owner。
