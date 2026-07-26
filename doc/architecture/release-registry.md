# Skiff Release / Registry Architecture

## 本文负责 / 不负责

本文定义dev sync/reload、production source revision、四类immutable artifact、typed pointer、
rollback和audit的长期边界。Package、ServiceContract、ServiceDeployment与RuntimeAssembly的对象语义以
[`package-service-contract-deployment.md`](package-service-contract-deployment.md)为最高权威；本文只负责
它们如何进入registry和release lifecycle。

本文不定义语言语法、YAML字段、artifact DTO细节、CLI拼写、端口或部署脚本。Skiff尚未发布，不保留
`PackageUnit`、`ServiceUnit`、`RuntimeProgram`、共同发布对象或旧pointer格式。

## 生命周期

Registry相关操作分成三条生命周期：

- Dev lifecycle：从本地source root编译canonical artifacts，写入dev artifact root，生成新的
  RuntimeAssembly并原子切换dev pointer，再由Router/Runtime reload。
- Package lifecycle：保存immutable source revision，经平台可信compiler生成PackageArtifact与schema
  records，并把人类坐标解析指针CAS到精确artifact。
- Service/release lifecycle：从同一个service Package生成ServiceContract、ServiceDeployment和环境
  RuntimeAssembly，完成admission后CAS deployment/assembly pointer。

三条生命周期使用相同typed artifact、identity、loader与linker语义，但信任边界不同。本地compiler产物只
能进入dev root；production record只能由平台可信compiler或验证过的artifact ingestion生成。

## Source Revision

Production可为一次Package源码快照创建immutable source revision，至少记录：

- Package id/version label；
- source snapshot与content hash；
- `package.yml`、`api.yml`、可选`service.yml`及配置声明hash；
- dependency resolution snapshot；
- compiler/toolchain provenance；
- publisher、时间和registry generation。

Source revision只用于可信重编译、审计和复现，不是Runtime加载对象，也不进入PackageArtifact、
ServiceProtocolIdentity或RuntimeAssembly identity。Runtime/Router不得读取source revision恢复缺失事实。

## Immutable Records

Registry分别存储：

- PackageArtifact及其File IR、PackageSchema record/index；
- ServiceContract；
- ServiceDeployment；
- RuntimeAssembly。

每个record先按canonical bytes计算identity，再完整写入content-addressed storage；写后必须重读并校验
identity。Immutable record永不原地覆盖。缺record、内容hash不符、owner不符或引用闭包不完整都fail
closed。

四种对象没有共同kind、共同父DTO或共同“发布单元”。一次workflow可以原子提交多个typed records，但这只是
事务编排，不创造第五种领域对象。

## Typed Pointers

可变状态只存在于显式typed pointer：

- Package coordinate pointer：`packageId + exact version label -> PackageArtifactId`，供compiler/tooling解析
  dependency；consumer artifact与RuntimeAssembly最终仍固定精确identity。
- Service deployment pointer：在`serviceId + exact version label + ServiceProtocolIdentity`下选择一个
  admitted ServiceDeployment revision。
- Environment assembly pointer：为某环境选择active RuntimeAssembly identity与generation。

Pointer更新必须CAS generation，并写append-only history。Pointer不能改变target record内容，也不能让
已经生成的RuntimeAssembly随“latest”漂移：

- Package pointer移动后，已有consumer/assembly保持原精确PackageArtifact；采用新artifact必须重新
  compile/link并生成新assembly。
- Service implementation可在protocol identity不变时切到新deployment revision；现有assembly仍固定旧
  revision，新流量采用新revision需要生成并激活新assembly generation。
- Protocol identity变化不能借相同version label静默迁移旧consumer。

## Production Workflow

Package workflow：

```text
immutable source revision
  -> trusted Package compile
  -> validate PackageArtifact + PackageSchema closure
  -> write immutable records
  -> CAS Package coordinate pointer
```

Service workflow：

```text
exact PackageArtifact
  -> project ServiceContract from explicit serviceCall roots
  -> project typed gateway entries and ServiceDeployment
  -> write immutable records
  -> CAS compatible ServiceDeployment pointer
```

Environment activation：

```text
explicit root deployment set
  -> close exact package/service dependencies
  -> generate and validate RuntimeAssembly
  -> materialize immutable artifact filesystem view
  -> Router prepare/Runtime admit
  -> atomic assembly-generation commit
```

失败必须发生在pointer移动前。Source不可解析、dependency缺失、schema/identity不闭合、operation或gateway
binding不完整、artifact写入不完整、runtime admission失败或CAS冲突都不能留下active半状态。

## Dev Sync / Reload

开发态不把本地source快照称为production publish。Dev sync仍必须生成与production相同shape和identity规则的
PackageArtifact、ServiceContract、ServiceDeployment与RuntimeAssembly；区别仅是：

- 输入可以来自当前工作区和dev profile；
- 输出进入隔离dev artifact root；
- dev pointer是可替换latest，不承诺正式history/compatibility policy；
- reload只观察已经完整写入并原子切换的assembly pointer。

Dev路径不得恢复旧Unit/RuntimeProgram writer、跳过identity/admission，或让Router/Runtime从源码和display
name补事实。

## Registry Service 与文件投影

生产durable source of truth可以由普通Skiff service `skiff.run/registry`实现。它没有compiler、runtime或
语言特权，只通过普通ServiceContract和DB capability工作。

Router与Runtime不调用registry service读取artifact。Registry/CLI把已经提交的immutable records和typed
pointers物化到部署配置的artifact filesystem：

- 先写并校验全部immutable blobs；
- 再原子更新pointer；
- Router只读取routing/assembly snapshot；
- Runtime按RuntimeAssembly引用读取精确闭包；
- 任何reader都不得接受半写入record或回退到旧格式。

## Rollback

Rollback只移动typed pointer，不删除历史：

- Package rollback把coordinate pointer CAS回旧PackageArtifact；只有新compile/link/assembly采用该选择。
- Service rollback选择旧的contract-compatible ServiceDeployment revision。
- Environment rollback生成或重新选择固定旧deployment closure的RuntimeAssembly，再按正常prepare/commit
  激活新generation。

Rollback不能在既有RuntimeAssembly内部替换PackageArtifact或ServiceDeployment。In-flight request、
server stream和WebSocket connection继续pin原generation，按drain规则完成或终止。

## Audit

每次production操作至少记录：

- source revision与publisher；
- compiler/toolchain provenance；
- immutable artifact identities与完整dependency closure；
- pointer old/new target、generation、发起者、时间与原因；
- ServiceProtocol/GatewayEntry identity校验；
- RuntimeAssembly admission、prepare/commit/abort结果；
- rollback目标与drain结果。

Audit用于回答“什么source产生了什么artifact”“某generation精确运行了什么”“为什么admission失败”和
“哪个pointer变化完成了rollback”，不得成为Runtime缺失identity的补充事实源。

## 不变量

- Production code artifact只来自可信compiler。
- Immutable record不修改；pointer可变但history append-only。
- Runtime执行只读精确RuntimeAssembly闭包，不动态解析latest Package/Service pointer。
- ServiceProtocolIdentity、GatewayEntryIdentity、deployment revision和assembly identity各自独立。
- Dev与production共享typed模型和验证，不共享信任边界。
- 缺selector、record、pointer、identity、admission或exact runtime registration时fail closed。
