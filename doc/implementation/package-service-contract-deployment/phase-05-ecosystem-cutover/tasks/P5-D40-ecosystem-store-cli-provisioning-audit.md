# P5-D40：Ecosystem Store CLI Provisioning Audit

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、8、9条、§12“RuntimeAssembly 与扩容”、§13
“Registry、Release 与 Publish”及§14“Fail-closed 条件”。Router必须消费canonical typed artifact/release状态，启动配置不得
回建Node store/parser或依赖ambient path；缺失、错误或不可执行的production store adapter必须在启动/调用边界fail closed。

DAG节点D40，依赖F03B发现instance/deploy脚本没有安装或写入Router `ecosystemStoreCliPath`，审计时F03B/F03C已合流。
本节点只返回事实、缺口、便宜探针与最小修复owner，不修改代码或给R05/R02/Phase verdict。风险高，验收分组为Router
canonical-store production provisioning。

全新只读Agent在派发的exact candidate建立以下闭合矩阵：

- `ecosystemStoreCliPath`的production定义、校验、spawn与所有test-only注入点；
- 从`skiff instance`/deploy/config normalize/render到Router child实际收到字段的完整配置链；
- compiler ecosystem-store CLI的build/install owner、dev-home/bin路径、可执行权限与worktree provenance；
- stable instance、isolated test instance与普通CLI启动是否共享或分离该配置，缺失/错误path如何fail closed；
- F03B Router consumer已覆盖与尚未覆盖的正反证据；
- 独立修复节点的最小写入边界、真实依赖、直接命令及哪些证据会失效。

只允许`rg`、`git show/diff/log`及源码/配置/既有测试静态读取；禁止编辑、提交、构建、测试、启动Router/runtime/instance、
操作stable、改本机配置或安装binary。不得建议ambient cwd/PATH fallback、Node store复刻或test-only默认值进入production。
若现有设计事实不足以决定CLI sidecar职责，明确标记为公共/架构决策；否则给出唯一implementation owner。

证据锚定派发时的exact HEAD/tree/Cargo.lock；scripts/config/Router store client/compiler CLI/build-install路径变化会使审计失效。
交付逐跳矩阵、blocker、owner和便宜探针后结束。
