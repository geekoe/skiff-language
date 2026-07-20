# P4-A01：Independent Stage Acceptance

## 角色与精确输入

未参与实现的只读阶段验收Agent。完整阅读权威设计§2、§6–§10、§12、§14、§15，`phase-plan.md`、全部P4任务
合同、R01–R03 verdict、T10覆盖矩阵/ledger，并检查T10冻结的exact clean commit/tree。

不得修改文件、创建commit、顺手修复或重复仍有效的昂贵gate。可运行风险所需聚焦抽查；按设计判断，不按开发
总结判断。

## 必验完成态

- assembly execution image无legacy aggregate，code shared且activation mutable owner隔离；
- package direct same-heap，service boundary detached且只用caller-relative InProcessBoundary；
- ordinary/error、async/stream/cancel、callback/native context/lifetime/error完整，no-provider无router fallback；
- callback opaque、request/stream scoped、persistent lane拒绝、不重建；
- ingress/internal single dispatcher与active-generation pin；request零artifact I/O/legacy route fallback；
- router不接受runtime-originated service relay，保留gateway/actor/spawn；
- execution checker真实覆盖owner/omission/rename/move/duplicate/TLS/shared-recoverable callback/remote relay；
- Phase 05 authoring/tooling边界未被兼容层偷渡，旧production service execution边不可达。

## 输出

第一行`PASS`或`FAIL`。列blocking issues、non-blocking follow-up、证据命令、动态测试缺口与残余风险，并说明每条
阶段标准由何真实入口/证据覆盖。PASS才允许主Agent合并main；verdict锚定exact candidate。

## 验收结论

PASS。锚定doc commit `c5b5d5e3359a7399fceeffd55078753b2b5f5f85` / tree
`0ace483a0e6f719f641cde862703825e45ecb0c5`与production candidate
`13b4600f38ae1d0cdc6878ecb518e2b616d5e4fa` / tree `a34e103cb8a95f0611b380ae3a173266471fcc6d`。
无blocking issue；11项阶段完成态均由production入口、结构负例与独立聚焦证据覆盖。Repo compiler-no-bin/fmt
失败确认为inherited baseline；registrations=0和Agine `/session` fail-closed确认为Phase 05动态缺口，未计作Phase 04
PASS证据。完整逐项结论、命令和non-blocking维护性后续见`../phase-result.md` §9。
