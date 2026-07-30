# P5-D43：Cargo.lock Refresh Audit

权威设计为
`doc/architecture/package-service-contract-deployment.md` §6.2、§7、§12及§14；执行顺序来自phase plan的Wave 2
shared-lock串行收口。

DAG节点D43，依赖R05B PASS。integration production commit为
`c59b4baf9752147cc49c141d89642d8b7f5aa507`，当前Cargo.lock blob为
`f3ce5457138c58aec4c84abda431afa96013e3fd`。root refresh事实：

- 裸`cargo generate-lockfile --offline`产生大量无关registry版本升级，已恢复且不得提交；
- `cargo generate-lockfile --locked --offline`失败，证明当前lock与manifests不一致；
- 工作区已恢复，除允许ledger外clean。

全新只读Agent必须：

- 沿git history定位最后一次有效Cargo.lock对应状态到当前HEAD之间所有workspace/root/member `Cargo.toml`变化；
- 区分Phase 5 owned manifest dependency metadata、后续无关manifest变化及registry resolver噪音；
- 确定最小lock delta应新增/修改的workspace package dependency edges与确有必要的新package records；
- 冻结一个开发owner可执行的命令/步骤，使既有registry package versions/checksums尽可能逐字保持，只更新owned
  manifest所需metadata；不得手工臆造checksum；
- 列出必须重建的Rust/compiler证据、仍有效的Router/scripts/R05证据与I02解除条件。

只允许`rg`、`git log/show/diff`、Cargo.toml/Cargo.lock静态读取及`cargo metadata --no-deps --format-version 1`
这类不修改lock的命令；禁止编辑、提交、generate/update、构建、测试、instance/stable。若无法在不升级无关registry
版本的前提下形成最小lock，报告精确阻塞与最小工具策略，不自行接受大diff。不作I02/R02 verdict。
