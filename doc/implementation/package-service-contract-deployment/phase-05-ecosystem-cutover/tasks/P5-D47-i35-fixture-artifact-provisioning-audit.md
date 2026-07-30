# P5-D47：I35 Fixture Artifact Provisioning Audit

权威设计为
`doc/architecture/package-service-contract-deployment.md` §3、§6.2、§7、§12–§14。

DAG节点D47，依赖I35/I35A连续两个invocation blocker。只读闭合normal-source fixture compile所需canonical std
PackageArtifact的hermetic provisioning入口；不作I35/R05C/I02/R02 verdict。

全新只读Agent在production candidate
`dada6d56a42d5eb917ec96db200fc2567b8195df`检查：

- `skiff test` source dependency resolver如何定位`skiff.run/std`，现有test/isolated owner如何seed canonical std；
- 是否已有单一CLI/helper能在临时artifact root构建/发布std，或应直接复用I02 fixture binary的完整authoring路径；
- empty root、stable root与isolated runtime seeded root的ownership差异；
- 第三次唯一fixture compile/test命令、cleanup、deadline及不触stable证明；
- 若缺现成入口，冻结最小test-infrastructure实现节点、direct test与combined失效面。

只允许`rg`、`git log/show/diff`及源码/既有测试静态读取；禁止编辑、提交、构建、测试、seed、instance/stable。
不得读取或复制stable artifact root。输出精确命令或最小implementation owner，并说明为何能提供canonical而非fake std。
