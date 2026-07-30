# Skiff Service Configuration Reference

## 1. Files And Scope

一个service activation从service root选择至多三份配置文件：

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
path、service id和display name都不能代替Package ID。未知Package ID必须在snapshot构造阶段失败。
普通Package root不拥有环境配置文件；它的值由最终宿主service这同一组文件中的Package ID分区提供。

Package源码只读取自己分区中的local dotted path：

```skiff
const cookieName = config.require<string>("cookieName")
const maxAge = config.optional<number>("maxAgeSeconds")
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
- overlay结果必须满足当前activation中每个精确Package build自己的typed requirements；
- `config.require<T>`的path缺失或类型不符使activation失败；
- `config.optional<T>`的path缺失返回`null`，存在但类型不符仍使activation失败。

同一个精确Package build经diamond dependency多次到达时，在同一service deployment中只有一份配置分区和
一份只读`ConfigView`。完整dependency alias和到达路径不参与配置identity。不同service deployment即使
使用同一个Package build，也必须拥有各自隔离的配置分区和`ConfigView`。

## 3. Secret File

`config.<profile>.secret.yml`与普通配置使用完全相同的Package-ID schema和overlay规则。它不是引用表，
不使用`SecretRef`，也不要求在普通配置中重复声明私密path。文件保存当前环境的明文值：

```yaml
"agine.ai/aihub":
  relay:
    apiKey: local-development-value
```

私密文件必须被版本控制忽略；tooling创建或复制它时必须使用仅owner可读写的`0600`权限，包含它的目录应为
`0700`。明文不得写入PackageArtifact、ServiceContract、ServiceDeployment、RuntimeAssembly、这些对象的
identity、receipt、control frame或日志。

第一版配置快照可以由受信runtime storage以明文保存。未来加密应作为独立snapshot store的整份envelope
能力加入；它不能重新引入字段级`SecretRef`，也不能改变本文件的authoring schema。本版本不定义KMS wire、
key provider或轮换协议。

## 4. Runtime Config Snapshot

所有业务配置值都进入独立的不可变`RuntimeConfigSnapshot`，不进入`ServiceDeployment`或
`RuntimeAssembly`：

```text
CommittedActivationGeneration
  ├── runtimeAssemblyRef
  └── runtimeConfigSnapshotRef
```

两个ref并列属于同一activation generation，彼此不引用。snapshot ID是随机、不透明、不可从内容或
artifact identity推导的immutable coordinate。snapshot内部至少按精确`ServiceDeploymentRef`隔离，再把
canonical Package ID解析到该deployment闭包中的精确Package build。Runtime只把匹配
`(ServiceDeploymentRef, PackageBuild)`的只读`ConfigView`交给对应执行slot。

配置变化创建新snapshot并提交新activation generation；它不重建PackageArtifact、ServiceDeployment或
RuntimeAssembly。冷恢复必须读取generation钉住的精确assembly ref和snapshot ref，不能读取目录中的
“最新配置”、ambient environment或另一个deployment的snapshot分区。

## 5. Platform Policy Is Not Business Config

这些文件只保存Package业务配置值，不保存平台部署策略。数据库名、数据库连接、principal、quota、
CPU/内存resource limit、runtime并发、activation timeout和request timeout都不属于本schema。

当前没有生效消费者的`state`、`principal`、`quota`、`resources`和deployment `timeout` profile字段必须
删除，不能为了满足artifact schema填写占位值。未来平台policy或resource配置必须由operator-owned独立
配置拥有，不能塞回Package业务配置文件。

这里删除的是service profile中的平台`resources` binding，不是`package.yml.resources`声明的Package静态
资源；静态资源仍随PackageArtifact发布。
