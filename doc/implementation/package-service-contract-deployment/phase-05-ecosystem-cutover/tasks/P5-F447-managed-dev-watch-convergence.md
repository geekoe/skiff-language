# P5-F447 Managed Dev Watch Convergence

## Authority

- [`managed-dev-watch.md`](../../../../architecture/managed-dev-watch.md)
- [`runtime-deployment-topology.md`](../../../../architecture/runtime-deployment-topology.md)
- [`config.md`](../../../../reference/config.md)
- [`P5-F446B-config-snapshot-tooling.md`](P5-F446B-config-snapshot-tooling.md)

本文只拆实现ownership和证据，不扩展production/stable rollout。

## External Review Disposition

| 审阅项 | 结论 | 本任务处理 |
| --- | --- | --- |
| managed watch启动固定`expectedGeneration = 0` | 成立，P1 | 启动读取Router exact committed tuple，保留CAS；launcher不再注入常量0 |
| dev sync未复制service config/secret YAML | 已过时 | 当前唯一链是`RuntimeConfigSnapshot`；禁止恢复旧文件复制或SecretRef |
| watch只在启动时读取registry | 成立，P1 | registry成为动态输入，每轮重读并进入语义fingerprint |
| sync失败后仍提交fingerprint、不会重试 | 成立，P1 | 仅完整成功后提交；pending按有界退避重试，新输入立即替换 |
| registry remove不能处理已删除root或service ID | 成立，P2 | v2持久metadata、结构读取与live验证分离、唯一匹配删除 |
| Runtime spawn负向测试仍在execution层失败 | 已迁移 | 当前测试已把link-time rejection与execution defensive check分开；不在本任务重复修改 |

## DAG

### [F447A Registry v2 And Canonical CLI](P5-F447A-registry-v2-canonical-cli.md)

Owner只修改CLI registry persistence与相关tooling tests：

- v2 entry持久`kind/root/serviceId?`，结构读取不触碰live root；
- add时live classify，remove按root或service ID唯一匹配，歧义fail closed；
- 同目录temp、file fsync、atomic rename与可用时directory fsync；
- canonical命令硬切为`skiff service dev registry`，删除错误的`skiff dev registry`分派；
- CLI usage、scripts README及测试fixture同步更新，不保留v1/旧命令兼容reader。

### [F447B Dynamic Watch And CAS Recovery](P5-F447B-dynamic-watch-cas-recovery.md)

依赖F447A的v2 DTO，但不拥有registry写入：

- managed launcher不再传`--expected-generation 0`；
- watch启动及CAS冲突后读取Router health exact committed tuple；
- registry每轮重读，effective environment/root集合进入语义fingerprint；
- bad/ENOENT registry保留last-known-good并失败重试；
- build + snapshot + activation commit后才提交last-success fingerprint；
- pending退避为`1/2/4/8/16/30s`并封顶，新fingerprint立即替换；
- 409只有在重读确认generation前进时才以新generation重试；目标exact pair已committed视为成功；
- 合法空registry发布canonical empty assembly + empty config snapshot并提交新generation；
- 不恢复配置YAML复制、SecretRef或无CAS activation。

### R447 Independent Acceptance

F447A/B合流后，由独立owner按
[`P5-R447-managed-dev-watch-acceptance.md`](P5-R447-managed-dev-watch-acceptance.md)
在同一候选验收。昂贵combined gate仍服从Phase 05唯一owner规则；本任务只运行tooling/router聚焦证据。

## Integration Boundary

- F447A/B可以从同一main checkpoint并行，文件重叠处由单一integration owner收敛。
- F447只修改Skiff tooling、Router health DTO及其测试，不修改Internals或official package源码。
- 不启动、重配或验证stable instance；stable rollout仍由Phase 05终端验收任务负责。
- 不为尚未发布的v1 registry、错误CLI路径或固定generation launcher保留兼容层。

## Developer Evidence

每个实现owner提交：

- 条款到代码/测试的映射；
- 受影响Node/Router type-check与聚焦测试；
- `skiff dev registry`、registry v1 reader、fixed generation 0和pre-success fingerprint update的反向搜索；
- `git diff --check`。
