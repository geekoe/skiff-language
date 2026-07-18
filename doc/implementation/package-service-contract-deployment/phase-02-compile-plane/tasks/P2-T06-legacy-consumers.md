# P2-T06：Legacy Runtime / Test Consumer Adapter

状态：cancelled；terminal-only 决策后没有对应交付物，不执行。

Phase 02 允许 runtime、package-test、test-runner、CLI 与 watch 暂时不可用，因此不需要、也不允许
`PackageArtifact -> PackageUnit/ServiceUnit/serviceAssembly` adapter。旧 integration 的 T06 提交不进入
新 branch；后续阶段直接实现 `ServiceDeployment`、`RuntimeAssembly` 与终态 consumer。
