# Skiff Service Configuration Reference

## 1. Files And Scope

发布一个service deployment时，tooling从service root选择至多三份配置文件：

```text
config.yml
config.<profile>.yml
config.<profile>.secret.yml
```

它们是一份service配置的三个overlay layer，不是每个Package各写一份文件。`config.yml`是可选base，
`config.<profile>.yml`是可选profile覆盖，`config.<profile>.secret.yml`是可选私密覆盖。不存在
`config:`、`service:`、`packages:`、`secrets:`等保留包装key；文件根直接以canonical Package ID为key：

```yaml
"agine.ai/api":
  model: qwen-plus

"skiff.run/http-session":
  cookieName: agine_session
  maxAgeSeconds: 2592000
```

Service首先是Package，因此service自身配置也使用自己的Package ID。Package dependency alias、source
path、service id和display name都不能代替Package ID。未知Package ID必须在deployment projection阶段失败。
普通Package root不拥有profile配置文件；它的值由最终宿主service这同一组文件中的Package ID分区提供。

Package源码只读取自己分区中的local dotted path：

```skiff
let cookieName = config.require<string>("cookieName")
let maxAge = config.optional<number>("maxAgeSeconds")
```

源码看不到Package ID包装层，也不能读取另一个Package的配置。`PackageArtifact`只保存当前Package源码
产生的typed config requirements；它不保存配置值，也不复制dependency Package的requirements。

## 2. Overlay

三层按以下顺序递归overlay：

```text
config.yml
  <- config.<profile>.yml
  <- config.<profile>.secret.yml
```

规则固定为：

- mapping与mapping递归合并；
- scalar和sequence整体替换旧值；
- 显式`null`是tombstone，删除该path而不是形成可读取的null配置值；
- overlay结果必须满足当前deployment package closure中每个精确Package build自己的typed requirements；
- `config.require<T>`的path缺失或类型不符使deployment build失败；
- `config.optional<T>`的path缺失返回`null`，存在但类型不符仍使deployment build失败。

同一个精确Package build经diamond dependency多次到达时，在同一service deployment中只有一份配置分区和
一份只读`ConfigView`。完整dependency alias和到达路径不参与配置identity。不同service deployment即使
使用同一个Package build，也必须拥有各自隔离的配置分区和`ConfigView`。

## 3. Secret File

`config.<profile>.secret.yml`与普通配置使用完全相同的Package-ID schema和overlay规则。它不是引用表，
不使用`SecretRef`，也不要求在普通配置中重复声明私密path。文件保存当前profile的明文值：

```yaml
"agine.ai/aihub":
  relay:
    apiKey: local-development-value
```

私密文件必须被版本控制忽略。在提供POSIX file mode与symlink语义的平台上，secret source必须是普通文件、
不得是symlink，且mode必须精确为`0600`；tooling必须在读取内容前检查并fail closed。任何tooling所需的
明文复制或暂存文件，写完后必须先设为`0600`并重新确认，再允许读取、overlay或publish。包含baked config的
受信deployment store目录保持`0700`，明文或可解密payload文件保持`0600`。

不提供POSIX mode的平台不能伪造同一检查结果；对应tooling/backend必须明确声明该边界，并使用平台等价的
owner-only ACL、拒绝link/reparse substitution及普通文件检查。没有已实现且可验证的等价安全边界时，secret
source读取必须fail closed。明文不得写入PackageArtifact、ServiceContract、release pointer、可选release
bundle、receipt、control frame或日志。它只允许进入ServiceDeployment-owned protected config payload；
公开descriptor、诊断与public content-hash preimage不得回显配置值。

第一版受信deployment storage可以在上述owner-only边界内保存明文payload。未来加密应作为artifact store的
整份protected envelope能力加入；它不能重新引入字段级`SecretRef`，也不能改变本文件的authoring schema。
本版本不定义KMS wire、key provider或轮换协议。

## 4. Baked Deployment Config

所有业务配置值都在发布时冻结给immutable ServiceDeployment：

```text
ServiceDeployment
  profile
  bakedConfigPayloadRef
  deployment buildId
```

`profile`由producer从受信operator输入取得；它不是从source YAML、service配置、路径或ambient environment
推断的业务值。Tooling把canonical Package ID解析到该deployment package closure中的精确Package build，
校验完整typed requirements，再写入immutable `BakedConfigPayload`。Deployment保存不可替换的opaque
protected ref；该ref参与buildId，但secret明文不进入公开hash preimage。Ref必须是store security
domain内确定性的keyed content identity（例如HMAC），或具有同等幂等与offline-guess
resistance的受信store identity。Store必须验证immutability与完整性；同一canonical payload
重复发布复用已验证ref，不因加密nonce改变buildId。Runtime只把匹配
`(deployment buildId, PackageBuild)`的`ConfigView`交给对应执行slot。

配置变化创建新的ServiceDeployment/buildId，但不重建PackageArtifact或ServiceContract。Publish原子更新
`(profile, serviceId, version) -> buildId` release pointer；rollback把pointer指回旧build。Runtime按exact
buildId懒加载时只读取该deployment固定ref下已经验证的payload，不能读取目录中的“最新配置”、ambient
environment或另一个deployment的配置分区。Payload没有独立pointer/current/commit lifecycle；不存在
`RuntimeConfigSnapshot`、prepare/cold-recovery配对或activation generation。

## 5. Platform Policy Is Not Business Config

这些文件只保存Package业务配置值，不保存平台部署策略。数据库名、数据库连接、principal、quota、
CPU/内存resource limit、runtime并发、deployment image load timeout和request timeout都不属于本schema。

`state`、`principal`、`quota`、`resources`和deployment `timeout`不是合法profile字段，不能为了满足
artifact schema填写占位值。`DeploymentPolicy`和`ResourcePolicy`不存在；external business request只受
Router operator配置的`requestTimeoutMs`平台上限约束，service配置不能覆盖。未来平台policy或resource
配置必须由operator-owned独立配置拥有，不能塞回Package业务配置文件。

这里删除的是未落地的deployment binding脚手架：`PackageResourceRequirement`、`ResourceBinding`、
`PackageRuntimeCapabilityRequirement`和`RuntimeCapabilityBinding`，不是`package.yml.resources`声明的
Package静态资源；`PackageArtifact.staticResources`、resource store/loader和`std.resource.*`继续保留。
Runtime内部真实使用的`NativeRequiredContext`、`NativeCapabilityContexts`，以及Router向Runtime协商
transport feature的`runtime.capabilities`也不属于这次删除范围。
