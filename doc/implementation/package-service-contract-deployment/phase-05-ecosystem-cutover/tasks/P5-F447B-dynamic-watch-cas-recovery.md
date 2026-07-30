# P5-F447B Dynamic Watch And CAS Recovery

## Parent

[`P5-F447-managed-dev-watch-convergence.md`](P5-F447-managed-dev-watch-convergence.md)

## Dependency

F447A registry v2 DTO与当前RuntimeAssembly/RuntimeConfigSnapshot exact activation pair。

## Scope

- managed watch首次activation前读取Router health exact committed environment/generation/pair；
- 删除launcher固定generation 0；CAS继续使用Router观察值；
- registry每轮重读，effective environment/root集合与root内容进入语义fingerprint；
- Router generation/exact pair只作为CAS观察状态，不进入期望状态fingerprint；
- bad/ENOENT/live-invalid registry保留last-known-good并自动重试，不解释为空；
- 只有build、snapshot publish与activation commit完整成功才提交last-success fingerprint；
- pending retry使用`1/2/4/8/16/30s`并封顶，新fingerprint立即替换；
- 409重读health：exact target已committed视为成功；只在generation确实前进后使用新值重试；
- 合法删除最后一个entry发布canonical empty assembly/config snapshot pair；
- 更新Router health exact tuple、managed launcher/watch tests与README，不复制YAML或恢复SecretRef。

本任务不修改registry writer，不启动stable instance，不扩展production rollout。

## Evidence

- nonzero启动generation、environment mismatch、409三分支；
- build/snapshot/activation各阶段失败自动重试且不提前提交fingerprint；
- retry被新fingerprint中断；
- dynamic add/remove/environment、bad-registry last-known-good与合法empty pair；
- tooling/Router聚焦tests、type-check、fixed-zero/pre-success-update反向搜索与`git diff --check`。
