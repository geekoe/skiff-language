# P5-G16D：V6 Real Host Gate Result

`G16D FAIL`

唯一full调用锚定`3ceb1cfa6a2f66b8b918a6df03718aaa40375e66` / tree
`b506f10a9d2e7f05e33e1c34b211e1b79b3e2626` / lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。这是D27/F21/P27R/R21C/I16闭合后新收敛周期的第1次、历史第4次
full-mode调用，也是历史第3次真实Host attempt；按当时Gate完成标准，完整positive Host累计仍为0。本Agent未重试。

preflight消费的v6 combined保持PASS且身份不变。full artifact evidence为PASS：四个targeted crate全Fresh，只有runner与
smoke两个top-level `.d`发生exact A→B root materialization，`changed=2/allowed=2/disallowed=0`。owned B dependency
install与tsx验证各执行一次，均code 0、signal null。真实child `node <B>/scripts/run-skiff-tests.mjs`同样code 0、signal
null，结果行精确为std `11 passed / 0 failed`与Host `1 passed / 0 failed`，共保留12条`PASS `前缀行。

Gate当时硬编码要求`PASS main.test.skiff::provider observes helper mutation`；该exact行计数为0，12条实际行均被归一为
`PASS <unexpected>`，故`hostAttempt.status`与顶层status保持FAIL，`sourceSuite`为null，唯一issue为`pass-line`。本checkpoint
不得因后续D30定位了false negative而把G16D改判PASS。

持久证据为
`/Users/geek/workspace/skiff-phase-05-evidence/p5-g16d-3ceb1cf-v6-real-host-gate.json`，文件SHA-256为
`a2c79614bddb3a3a2c326d7e6422e29d8d11cfc4e91046669d5a7d3cb606ab91`，内部ledger digest为
`1b345e59efaa472cdb226081179117ebbaf68478b2c92f8314d5367f95f5f170`。A/B worktree及Git admin/registry、owned task
root/shared target、24个process group与46865–46867端口全部ABSENT，foreign state preserved，cleanup errors为空；
`stableOperations: 0`。失败退回D30审计与新的修复DAG，不给F04或阶段verdict。
