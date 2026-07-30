# P5-R22：Host Result Evidence Acceptance Result

`R22 FAIL`

## 候选与范围

- candidate：`9de99ab547da68813235a3de97c3772f201484ad`
- tree：`3eea433a18c3d31d727b846393f7d6edf184fc84`
- `Cargo.lock` blob：`f3ce5457138c58aec4c84abda431afa96013e3fd`
- tracked clean before/after；修改、提交、Cargo、dependency install、I16、Host、full、runtime、stable均为0。

全新只读 reviewer完成P5-R22矩阵、focused tests、shared-target窄测、静态/反搜和三项合同内存probe。拟定FAIL只含
一个精确blocker；没有公共设计或业务语义问题。

## 唯一 blocker

`platform-source-probe-host-evidence.mjs`只统计位于stdout Host segment内的exact-test-name PASS。若Host segment已有
一条合法目标，同时std result之前、Host result之后或stderr另有同名合法PASS，外部同名行被当作unrelated hash，
最终仍得到`status:PASS`、`exactPassLineCount:1`与非空`sourceSuite`。

reviewer用production owner构造三个不写文件的最小复现，均错误PASS：

1. Host段目标 + std前同名PASS；
2. Host段目标 + 第二条result后同名PASS；
3. Host段目标 + stderr同名PASS。

这违反R22“exact test name全输出唯一，且唯一项必须位于stdout Host segment”的完成条件，会让歧义identity生成正式
final-value evidence。最小修复owner是同一test-only Host evidence模块：先统计stdout/stderr全部语法合法且test name精确
匹配的identity，要求总数恰好1，再要求该唯一项位于stdout Host segment；补上述三个组合负例。不得改变module spelling、
runner/fixture或公共语义。

## 其余证据

- focused Host evidence：`21 passed / 0 failed`；shared-target合同pattern：`19 passed / 0 failed`。
- 三个`node --check`与`git diff --check`均PASS。
- alternate合法module单独happy通过；目标仅在Host后或仅stderr且Host段无目标时正确FAIL。
- 单一owner/薄re-export、无module literal、identity grammar、11/11→1/1、fixture/assertion派生、unexpected SHA token、
  v6字段集合、diagnostic与primary-before-cleanup均PASS。
- extra-review除上述blocker外无维护性finding。

本FAIL只解锁P5-F22B及原R22 reviewer对同一精确blocker的复验，不解锁I16、G16E、R23或F04 receive。
