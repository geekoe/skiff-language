# 单一 Profile 激活模型与 `skiff stack` 部署

> 本文件是当前实现阶段的唯一权威设计文档。它替换旧文档中所有把 `environment`
> 作为独立激活标识的语义；实现以本文件为准，其余文档在阶段 E 同步更新。
> Skiff 尚未发布，不保留旧 `environment` wire/artifact 兼容层。

## 1. 目标

1. 删除 `environment` 这一独立概念，用 `profile` 同时承担配置 overlay 选择与激活世界标识。
2. 新增 `skiff stack` 命令族，以 `--configDir <dir>` 管理远程部署；配置目录内的
   `router.yml` / `runtime.yml` / `telemetry.yml` 直接复制，不做生成。
3. `skiff instance` 保持本地实例专用配置，不并入 stack 配置；watch 仅属于本地。
4. 单环境承诺：一个部署只有一个 profile，不引入多环境状态机。

## 2. 标识模型

- 删除 `environment` 作为独立领域概念。唯一标识为 `profile`，token 规则统一为
  现有 activation-environment 模式 `[A-Za-z0-9._-]{1,200}` 且显式拒绝 `.` 与 `..`；
  Router 的 `is_valid_profile`
  与 config-snapshot tooling/CLI 校验收敛到同一 validator。
- `profile` 同时决定：
  1. 每个 service root 的配置 overlay 选择（`config.yml` → `config.<profile>.yml` → `config.<profile>.secret.yml`）；
  2. Router 启动时读取的 committed activation state key；
  3. RuntimeConfigSnapshot 的校验标识；
  4. 工具层 CLI 参数（`--profile`）。
- 单环境承诺：一个部署一个 profile。不实现多环境切换、profile 列表或跨环境迁移。
- 唯一例外：test-runner 保留激活 target 与 config profile 的分离
  （test service 的 config profile 固定 `skiff-test`，激活 target 不用于选择 config 文件）。

## 3. 配置语义

### 3.1 router.yml

- 只保留顶层 `profile`（必需）。
- 删除顶层 `environment`。RouterConfig 不再持有 environment 字段。

### 3.2 runtime.yml

- 删除 `environment`（及任何 profile 本地值）。
- Runtime 的 profile、artifactsPath、serviceDb、http 限制全部由 Router 在连接级
  bootstrap 时下发；runtime.yml 只保留进程本地事实
  （router URL、runtime-home、keyring、`http.egress.proxy`）。

### 3.3 telemetry.yml

- 不变（当前无 environment 字段）。

### 3.4 service 配置

- `config.<profile>.yml` / `config.<profile>.secret.yml` 的选择只由 profile 决定；
  不再存在 environment 与 profile 的绑定或分离。

## 4. Router 语义

- `RouterConfig.environment` 删除；`profile` 保留为必需字段。
- E-bootstrap：从 Mongo `skiff-router.activation_state` 读取
  `_id == profile` 的 committed state，严格加载
  RuntimeAssembly + RuntimeConfigSnapshot + actor routing projection，
  发布 routing epoch 后才绑定监听；缺失/损坏/identity 不匹配一律 fail closed。
- Router→Runtime 连接级 bootstrap frame：`activation.environment` 改为
  `activation.profile`，随 artifactsPath、serviceDb、http 限制一起下发。
- assembly activation request：`environment` 字段改为 `profile`；
  schema `skiff-assembly-activation-request-v2` → `v3`。
- activation state repository：key 从 environment 改为 profile；
  audit 与 CAS 语义不变。

## 5. Runtime 语义

- 进程不再读取/配置 environment；连接级 bootstrap 接收 profile。
- 物化任何 ConfigView 前校验 `snapshot.profile == bootstrap.profile`；
  不一致 fail closed，不读取 ambient 值。
- 受信 profile 生命周期：首次 bootstrap 时冻结；后续 bootstrap 与冻结值不一致
  时 fail closed（重连报错），不静默重载。
- `http.egress.proxy` 保留在 runtime.yml（进程本地网络事实），不进入 bootstrap frame。
- 请求 frame、actor/dispatch、WebSocket 继续携带精确 assembly identity +
  generation + deployment revision；profile 只出现在 bootstrap 与快照校验层。

## 6. 快照与激活状态

### 6.1 RuntimeConfigSnapshot

- 字段 `environment` 改名为 `profile`。
- record schema `skiff-runtime-config-snapshot-record-v2` → `v3`；
  corpus/golden 同步更新。RuntimeConfigSnapshotId 保持随机 UUID，不改为内容寻址。

### 6.2 Activation state

- `EnvironmentActivationState` 系列 wire 字段改为 `profile`，schema 使用新命名空间
  `skiff-profile-activation-state-v1`（不是 v2→v3 升级）。
- committed/pending 结构与 generation/CAS 语义不变。
- Mongo `skiff-router.activation_state`：`_id = profile`，
  `state.profile` 唯一索引与 audit 索引同步改名；旧集合在阶段 D 全量重置。

### 6.3 Artifact 文件路径

- `environments/<env>/activation.json` → `profiles/<profile>/activation.json`。
- runtime assembly pointer 的 `release` key 由工具传 profile 值；
  其余 content-addressed record 路径不包含标识，保持不变。

## 7. 工具层

- CLI `--environment` 全部改为 `--profile`：
  `skiff dev sync/watch`、`skiff assembly build/publish/activate`、
  `skiff package build/publish`、config-snapshot-tooling、测试 fixture 与 live harness。
- dev sync 删除 `profile = environment` 绑定，直接传 profile。
- instance 配置 `environment: dev` 改为 `profile: dev`；
  `.stack/` 作为本地默认 configDir/状态目录，不再使用 `.skiff-instance`。
- watch 仅存在于本地 instance/dev；stack 命令族不提供 watch。
- durable task 的 `TaskExecutionImageRef.target_environment` 改为
  `target_profile`；持久化 schema 随阶段 D 重置迁移。

## 8. `skiff stack`

### 8.1 命令

```text
skiff stack build    --configDir <dir>   # 本地交叉编译 runtime stack
skiff stack init     --configDir <dir>   # 全新主机 bootstrap（正式 tooling）
skiff stack deploy   --configDir <dir>   # 上传二进制 + 复制 YAML + PM2
skiff stack status   --configDir <dir>   # ssh + /__router/health 验证
skiff stack validate --configDir <dir>   # 配置/文件一致性校验
```

### 8.2 配置目录内容

```text
<configDir>/
  build.yml         # target、zigDir、buildRoot、cargoTargetDir、可选 units
  config.yml        # profile、remote、verify（见下）
  router.yml        # 直接复制到远端 config/router.yml
  runtime.yml       # 直接复制到远端 config/runtime.yml
  telemetry.yml     # 直接复制到远端 config/telemetry.yml
```

`config.yml` 只放三个 YAML 之外的事实：

```yaml
profile: prod

remote:
  host: root@skiff.hanzhe.com
  remoteSkiff: /root/skiff
  nodeBin: /root/.local/share/fnm/node-versions/v22.22.1/installation/bin
  serviceDbKeyringFile: /run/secrets/skiff-service-db-keyring.json   # 可选，只在远端 provision

verify:
  httpPort: 4000
  controlPort: 4001
  telemetryPort: 4002
  healthPath: /__router/health
```

### 8.3 profile 同步机制

- 事实源是配置目录内的文件本身，不存在生成，因此没有第二个来源。
- `config.yml.profile` 与 `router.yml.profile` 必须相等；
  `validate`、`deploy`、`init` 在启动时强制检查，不一致 fail closed。
- `status` 交叉核对远端 router.yml、health 返回的 profile 与 config.yml 三者一致。
- runtime.yml 不写 profile（由 Router 下发），避免第三份拷贝。

### 8.4 build

- 只读 `build.yml`，调用 `build-runtime-stack.mjs` 的现有逻辑；
  构建始终发生在本地开发机，不依赖目标。

### 8.5 init（正式 bootstrap）

- 用正式 compiler + config-snapshot tooling 生成空 RuntimeAssembly、
  profile 已设置（与 config.yml 一致）、deployments 为空的 RuntimeConfigSnapshot、
  std records 与 actor routing projection；
  不再使用 `skiff-package-service-smoke-fixture` 作为生产引导入口。
- 物化到远端 `remoteSkiff/artifacts`，初始化 Mongo
  `skiff-router.activation_state`（`_id = profile`，generation 0），然后启动 router。
- 仅在全新主机或重置环境时使用一次。
- 实现位置：`skiff stack init` 命令，复用现有 authoring 库
  （compiler authoring + config-snapshot authoring），不新增独立 Rust binary。

### 8.6 deploy

- 删除现有 `deploy-runtime-stack.mjs` 的 YAML 渲染逻辑；
  改为从 `--configDir` 原样复制三个 YAML 并上传二进制、安装 telemetry 依赖、PM2 reload。
- 保留可被 CI/测试直接调用的底层模块边界。

## 9. 非目标

- 不引入多环境、多 profile 切换、profile 列表或跨 profile 迁移。
- 不保留旧 `environment` wire/artifact/CLI 兼容层或 alias。
- `skiff instance` 本地配置不并入 stack 配置目录。
- stack 不提供 watch；watch 仍是本地 dev/instance 能力。

## 10. 阶段与纵向验收

每个阶段完成后必须能运行真实纵向路径，不能只证明中间字段。

- 阶段 A（核心契约）：artifact-model / runtime-config-snapshot / deployment /
  artifact-identity 的 wire、路径、schema 改名；单元测试与 corpus/golden 全绿；
  验收按 §12 限定路径搜索 + 白名单，不允许全局 `rg environment` 作为完成标准
  （test 语料允许仅剩历史注释）。
- 阶段 B（Router/Runtime）：隔离或本地真实链路 bootstrap 成功，
  health 返回 profile、runtime connected healthy。
- 阶段 C（工具层）：`skiff dev/assembly/package` 以 `--profile` 跑通本地真实链路。
- 阶段 D（skiff stack）：`build → init → deploy → status` 在远端真实主机跑通；
  health generation 0；发布一个最小 service 后 activation generation 1 可见。
- 阶段 E（文档与收尾）：既有文档同步、独立验收、按授权合并 main。

## 11. 风险

- wire/schema/identity 变更会失效大量 golden 与 corpus，必须批量更新而非逐例修补。
- 远端现有部署在阶段 D 全量重置：activation state、artifacts、durable task 记录；
  service-db 加密域跟随 profile，旧加密数据重置后不可读（单用户阶段已确认可重置）。
- 共享主 worktree 只读；所有实现/集成在独立 worktree 与分支进行。

## 12. 评审决议（2026-08-04，用户确认）

- wire/schema 版本：assembly activation request v2→v3；RuntimeConfigSnapshot
  record v2→v3；activation state 新命名空间 `skiff-profile-activation-state-v1`；
  Router→Runtime frame `skiff-runtime-frame-v3`→v4（`activation.environment` →
  `activation.profile`）。
- 完整 wire 清单：bootstrap frame、AssemblyActivationControl 各变体、
  health wire（`activeAssembly.profile`、`replicas[].profile`）、Mongo state/audit
  索引、durable task `target_profile`，全部随本设计改名。
- 阶段 A 验收改为限定路径搜索 + 白名单（不包含 OS env、编译器词法等无关用途），
  不允许全局 `rg environment` 作为完成标准。
