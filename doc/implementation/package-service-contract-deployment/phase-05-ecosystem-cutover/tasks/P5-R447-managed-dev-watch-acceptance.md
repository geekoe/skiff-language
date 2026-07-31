PASS

# P5-R447 Managed Dev Watch Acceptance

## Authority

- [`P5-F447-managed-dev-watch-convergence.md`](P5-F447-managed-dev-watch-convergence.md)
- [`managed-dev-watch.md`](../../../../architecture/managed-dev-watch.md)

本任务是只读独立验收，不执行stable rollout。

## Acceptance Matrix

| 场景 | 必须观察到 |
| --- | --- |
| Router committed generation非0后启动watch | 第一次activation使用health返回的当前generation，不使用0 |
| health environment与registry effective environment不同 | fail closed；不构建或提交到错误environment |
| 普通source/config变更 | exact assembly/snapshot按需产生，成功commit后才推进last-success fingerprint |
| build、snapshot publish、Router连接或Runtime prepare暂时失败 | 原fingerprint保持pending，按`1/2/4/8/16/30s`自动重试，无需再次编辑文件 |
| 退避期间出现新输入 | 新fingerprint立即替换旧pending并立即尝试，旧目标不继续排队 |
| activation返回409且generation已前进 | 重读health，以新generation有界重试；不盲增本地值 |
| activation返回409且目标exact pair已committed | 视为幂等成功并提交last-success fingerprint |
| activation返回409但generation未前进或health不完整 | fail/retry，不用同一expected generation紧循环 |
| watch运行中registry add/remove/environment变化 | 无需重启watch即重新投影effective roots/environment |
| registry临时ENOENT、损坏或live root非法 | 保持last-known-good committed pair；不得投影为空；修复后自动恢复 |
| remove已经从磁盘删除的root | 结构读取成功并按持久root唯一删除 |
| remove使用service ID | 唯一命中才删除；零命中、重复或root/service ID解释冲突均fail closed |
| registry writer中途失败 | 原文件仍完整；不会暴露截断JSON或跨目录rename窗口 |
| 显式移除最后一个合法service | 发布canonical empty assembly和empty config snapshot，commit新generation并撤下旧services |
| config/secret输入 | 只进入secure RuntimeConfigSnapshot；artifact root不存在旧YAML复制和SecretRef恢复 |
| Runtime spawn负向测试 | link-time非法目标由linker测试拥有；execution defensive check保持独立通过 |

## Static Gates

- registry v2只接受canonical字段、绝对root、排序与唯一性规则；
- production source没有registry v1 reader或`skiff dev registry`兼容分派；
- managed instance/watch没有固定`expectedGeneration: 0`或等价参数注入；
- Router generation/exact committed tuple只作为CAS观察状态，不进入期望状态fingerprint；
- fingerprint只在完整activation成功路径提交；
- retry timer封顶30秒且新fingerprint能中断旧等待；
- empty root合法路径与bad-registry路径是不同测试分支；
- README、CLI usage与实现命令一致；
- `git diff --check`通过。

最终记录必须锚定exact commit/tree与实际命令结果；不能用本设计文档或历史审阅代替实现证据。

## Result Record

验收锚定commit `8405a206a2f79dc8a70b36bdfec334a9fa293f9f` / tree
`487a05f452e5204da1900ed48b95801e8fb52a1d`，后续canonical YAML parser收口至
`206e8c25adbece46de0a3ab204eff00bbe853c95` / tree
`85ddefa3430522af8116be81925a30c3ebbf98cd`。

managed watch/registry Node组合测试`94/94`，最终三文件聚焦`38/38`，registry parser`25/25`；canonical
empty assembly/snapshot的compiler、deployment、loader和Host聚焦测试全部PASS。脚本语法、package-store
discovery、command policy、Rust格式与diff检查PASS，registry v1和固定generation 0生产残留为0。

stable最终运行generation 12；watch、Router和Runtime均在线，active pair一致，pending activation为null。
结论：R447 **PASS**。
