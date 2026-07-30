# P5-I35C：Spawn Submit Fixture Post-repair Acceptance

DAG节点I35C，依赖F46A及I36 PASS。它是明确test-runner blocker修复后的新candidate复验；只有真实fixture
readiness→test执行能建立缺失证据，不能用unit test替代。

exact production candidate为commit
`95296242921cf26dfe961a735f652a84caf249b4`、tree
`2768f0822ed68ad511723442a45604e18a32c115`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

全新只读Agent逐字执行
`P5-D47-i35-fixture-artifact-provisioning-audit-result.md`中的唯一完整命令一次，随后`git diff --check`一次。
禁止试跑、重试、重复I35其它证据、编辑、提交、真实R05/I02、instance/stable/full gate。

第一行`I35C PASS`或`I35C FAIL`；必须报告bootstrap、compile、readiness、tests、cleanup及candidate状态。PASS与I35/I35A/B
仍有效证据共同关闭I35并解除R05C；FAIL停止该路径，不继续完整probe。
