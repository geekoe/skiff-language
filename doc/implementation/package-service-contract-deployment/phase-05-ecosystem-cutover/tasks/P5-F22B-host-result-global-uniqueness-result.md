# P5-F22B：Host Result Global Uniqueness Result

`F22B PASS`

- dev commit：`a0ebe4f103b344d2ce1f617403739cbf6c0a3fc8`
- integration commit：`104c8ee8e4719c14a5a0b330f300df5f62092ae8`
- tree：`48c0e8055522e244759de7b9497154bc5945a6f6`
- `Cargo.lock` blob：`f3ce5457138c58aec4c84abda431afa96013e3fd`

exact-test-name identity现在先在全部stdout/stderr中计数，再要求总数恰好1且唯一项位于stdout Host segment；
`exactPassLineCount`记录全局数。正确Host目标叠加std前、Host后、stderr同名行的三个组合均FAIL，count为2、
`observedPassLine`与`sourceSuite`为null，两行只保留SHA token；alternate合法module唯一正例继续PASS。v6字段集合、
11/11→1/1、process/port、fixture/assertion/finalValue与module-agnostic语义不变。

开发聚焦测试`26/26`、两个Node syntax check与diff-check均PASS。合流后root便宜combined精确运行Host evidence、diagnostic、
shared-target、negative与source-suite五文件，结果`80 passed / 0 failed / 0 skipped / 0 cancelled / 0 todo`，diff-check PASS。
未运行Cargo、dependency install、I16、Host/full、runtime或stable；无公共设计问题。

该PASS只解锁原R22 reviewer对其唯一global-uniqueness blocker的P5-R22B复验，不直接解锁I16/G16E。
