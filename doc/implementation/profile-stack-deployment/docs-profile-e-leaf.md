# Leaf Task: 阶段 E 既有文档同步为 profile 语义（docs-profile-e）

## 引用链

- 权威设计：`doc/architecture/profile-stack-deployment.md`
  （integration/profile-stack @ 1d4ac521 已提交；本文件禁止修改）。
- 直接父节点：设计 §12（阶段 E：既有文档同步）。
- 集成 Agent：`skiff_integration`（集成分支 integration/profile-stack，HEAD 1d4ac521）。
- 本分支：dev/docs-profile，worktree
  `/Users/geek/workspace/skiff-docs-profile`。

## 零 worktree 只读预检结论（基线 1d4ac521，Git 对象锚定）

### 1. 基线状态

- `integration/profile-stack` HEAD == 1d4ac521，与任务 baseline 一致。
- 共享主 worktree（main）未受影响；并行 worktree：`skiff-profile-stack-integration`
  （集成 Agent）、`skiff-dev-testinfra` / `skiff-integration-testinfra`（无关批次）、
  `skiff-dev-stack-cmd`（阶段 D 节点，另见 /root/dev_stack_cmd）。本节点写集与它们无重叠。
- 预检方式：`git archive 1d4ac521` 物化到临时目录后 `rg`，全程零 worktree 写操作。

### 2. 待改文档与行（rg 锚定 1d4ac521，按文件）

```text
AGENTS.md                                           223  --environment <environment>
router/README.md                                    41   environment: dev（示例）
                                                    76   environment 为必需字段
                                                    225  Runtime frame v2 / skiff-runtime-frame-v2
runtime/README.md                                   6    its environment
                                                    29   environment: production（示例）
                                                    35   environment 为必需字段
doc/overview.md                                     91   environment 与 service identity 派生数据库
doc/reference/config.md                             26   环境配置文件
                                                    66   当前环境的明文值
                                                    101  targetEnvironment
                                                    108  snapshot.targetEnvironment == activation.environment
doc/reference/runtime.md                            130  runtime frame v3 / v3 activation control
                                                    139  activation environment 与 serviceId 定界
                                                    148  targetEnvironment 与 activation environment 校验
doc/reference/service-yml.md                        31   环境配置
                                                    51   live target environment 不改变该 profile（测试例外）
doc/reference/testing.md                            63-68 测试配置 profile 与 runtime 目标 environment（§2 例外）
                                                    185-186 target environment / activation environment
                                                    278  target environment 与 expected generation
                                                    415  target environment/generation
doc/reference/db.md                                 512  storageEnvironment（AAD 上下文）
                                                    647  environment 从 Runtime config 读取
                                                    650  environment + serviceId 推导 target
                                                    655  environment + serviceId + ... 重加密上下文
                                                    667  Runtime config 是 environment 与 keyring 唯一来源
doc/architecture/release-registry.md                65   Environment assembly pointer
                                                    100  Environment activation
                                                    116  environment roots
                                                    153  Environment rollback
doc/architecture/managed-dev-watch.md               11   registry 的 environment
                                                    19   effective environment（两处）
                                                    27   committed environment
                                                    38   "environment": "dev"（registry JSON）
                                                    88   同 environment 的 empty snapshot
                                                    100  health tuple 的 environment
                                                    106  Router environment 与 effective environment
                                                    122  environment 不符
doc/architecture/runtime-deployment-topology.md     16   environment 只有一个 active RuntimeAssembly
                                                    27   environment 与 service identity 定界
                                                    45-48 targetEnvironment == activation.environment
                                                    70   storage domain/environment/service identity
                                                    85   ActiveAssemblyContextSet key 的 environment
                                                    129  target environment（TaskExecutionImageRef）
                                                    175  environment 与 serviceId 定界
                                                    208  environment root set
doc/architecture/durable-task-dispatch.md           72   owner、environment、execution image
                                                    138  targetEnvironment
doc/architecture/router-rust.md                     80   RoutingEpoch（environment、...）
doc/architecture/gateway-runtime-adapter-boundary.md 60  skiff-runtime-frame-v3 / v3 activation control
doc/architecture/runtime-compiler-shared-artifact-types.md
                                                    100  trusted target environment
                                                    105  target environment
                                                    106  activation environment
                                                    176  target environment
doc/architecture/db-capability-architecture.md      15   environment 与 serviceId 定界
                                                    202  (storage domain, environment, serviceId)
doc/architecture/open-issues.md                     38   environment 与 serviceId 定界
doc/architecture/test-runner-runtime-isolation.md   57   snapshot 顶层 target environment
                                                    58   snapshot target 与 activation environment
                                                    125  SKIFF_TEST_ENVIRONMENT（OS env 键，保留）
doc/architecture/runtime-layered-crate-architecture.md
                                                    231  activation environment
                                                    764  (environment, RuntimeAssemblyRef, ...)
doc/architecture/package-service-contract-deployment.md
                                                    538  Router↔Runtime frame schema v3
                                                    1229 targetEnvironment
                                                    1235 snapshot.targetEnvironment == activation.environment
                                                    1239 environment 与 serviceId 定界
                                                    1244 environment 变化
                                                    1259 (trusted storage domain, environment, serviceId)
                                                    1322 当前 environment 的精确 deployment refs
                                                    1326 environment assembly
                                                    1348 assembly-per-environment
                                                    1385 environment 与 service identity 定界
                                                    1388 environment activation prepare/commit/abort
```

### 3. 排除项（非本节点写集）

- `doc/architecture/profile-stack-deployment.md`：权威设计，逐字节不变。
- `doc/implementation/**`：历史实现记录与阶段结果/复盘保留原样；
  `profile-stack-deployment/` 下仅新增本叶子文件。
- 代码、配置、fixtures 与测试：不改。
- 通用 OS 环境语义：`environment variable`、`Environment proxy settings`、
  `host environment`、`ambient environment`、`local service environment`
  （scripts/README.md、telemetry/README.md、file-and-command.md、
  db-capability-architecture.md:254、runtime/README.md:124/130、
  runtime-deployment-topology.md:142、testing.md:191、config.md:110 等）保留。
- `SKIFF_TEST_ENVIRONMENT`：OS env 键，按阶段 C 决议（§12 白名单）保留原名；
  仅同步周围“target environment”表述为 profile。

## 任务边界与实现决策

### 写集

1. CLI/配置示例：AGENTS.md `--environment` → `--profile`；router/README.md 删除
   `environment: dev` 并将必需字段改为 profile；runtime/README.md 删除
   `environment: production` 并将必需字段收敛为 router/runtime-home
   （设计 §3.2：runtime.yml 不携带激活标识）。
2. wire/schema：runtime frame v3 → v4（router/README、runtime.md、
   gateway-runtime-adapter-boundary.md、package-service-contract-deployment.md 表）。
3. snapshot/activation 校验：`targetEnvironment` → `profile`、
   `snapshot.targetEnvironment == activation.environment` →
   `snapshot.profile == activation.profile`（config.md、runtime.md、
   runtime-deployment-topology.md、runtime-compiler-shared-artifact-types.md、
   package-service-contract-deployment.md、test-runner-runtime-isolation.md、testing.md）。
4. storage/加密域：`storageEnvironment` → `storageProfile`；
   `environment + serviceId` / storage domain 定界表述 → profile
   （db.md、db-capability-architecture.md、open-issues.md、overview.md、
   runtime.md、runtime-deployment-topology.md、package-service-contract-deployment.md）。
5. durable task：`targetEnvironment` → `targetProfile`，
   “owner、environment、execution image” → profile（durable-task-dispatch.md、
   runtime-deployment-topology.md）。
6. dev watch / registry / release：registry JSON `"environment"` → `"profile"`、
   effective/committed environment → profile、Environment assembly
   pointer/activation/rollback → Profile（managed-dev-watch.md、release-registry.md）。
7. routing/上下文：RoutingEpoch、ActiveAssemblyContextSet key、activation
   environment → profile（router-rust.md、runtime-layered-crate-architecture.md）。
8. 测试体系例外（设计 §2，必须保留语义）：testing.md 与 service-yml.md 改为
   “config profile 固定 `skiff-test`、激活 target（`--profile`）独立”，
   激活 target 不得反向选择 `config.<profile>.yml`。

### 强制停止

- 需修改设计文档、doc/implementation 历史记录、代码或配置时停止上报。
- 对语义存在歧义（如测试例外、OS env 键、migration-tool 操作面字段名）时
  按设计 §2/§12 与基线代码事实判断；超出设计范围的变化不自行补设计。

## 自验收矩阵（同步完成后回填）

| 设计/任务条款 | 文档证据 | 反向搜索证据 | 验收命令 |
| --- | --- | --- | --- |
| CLI/示例 environment → profile | AGENTS.md、router/README.md、runtime/README.md | `rg -- '--environment' doc/` 零残留（除测试负例/历史白名单） | rg |
| frame v4 / request v3 / snapshot record v3 / activation state v1 表述一致 | router/README、runtime.md、gateway、package-service 表 | `rg 'frame-v3|targetEnvironment|activation\.environment' doc/` 零残留（白名单除外） | rg |
| storage/加密域 profile | db.md、db-capability-architecture.md、overview.md 等 | `rg 'storageEnvironment|environment \+ serviceId' doc/` 零残留 | rg |
| 测试体系例外保留 | testing.md、service-yml.md 表述 | `skiff-test` 固定配置 + 激活 target 独立语义仍在 | rg + 人工核对 |
| 历史实现记录与设计文档未动 | — | `git diff --stat 1d4ac521..HEAD -- doc/implementation doc/architecture/profile-stack-deployment.md` 仅含本叶子 | git diff |

## 交接

完成后提交到 dev/docs-profile，报告给 `skiff_integration`：
branch、worktree 路径、commit/tree、实际写集、自验收证据与命令、剩余风险。
