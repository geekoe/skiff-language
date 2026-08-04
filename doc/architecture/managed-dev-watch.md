# Managed Dev Watch

本文定义开发态managed watch如何把动态service root集合投影为
`RuntimeAssembly + RuntimeConfigSnapshot`，并通过Router activation CAS持续收敛。它是tooling/control
plane契约，不是语言语义，也不定义production平台发布流程。

## 输入与状态

Managed watch的期望状态由以下输入共同决定：

- dev registry的schema版本、profile和entries；
- 命令行显式追加的静态root；
- effective root集合的源码、control files、三层配置和Package解析输入。

Router当前committed activation tuple是CAS观察状态，不是期望状态输入，也不进入语义fingerprint；否则
watch自己的每次成功activation都会凭generation变化再次触发下一次activation。

watch不能只在进程启动时展开一次registry。每轮poll及registry文件变化后都必须重新读取registry，重新计算
effective profile与root集合，再验证live roots。registry schema、effective profile、canonical
entry/root集合及root内容都进入同一个语义fingerprint；只比较启动时root的mtime不是合法实现。

watch内部至少区分：

- `lastKnownGoodRegistry`：最近一次结构合法且live root验证通过的registry；
- `pendingFingerprint`：当前等待构建或重试的期望状态；
- `lastSuccessfulFingerprint`：已经完成build、snapshot publish和activation commit的状态；
- Router返回的exact committed profile、generation、assembly ref与config snapshot ref。

这些状态不能折叠为一个“最近看过的fingerprint”。失败的输入从未成功生效，因此不能被标记为已处理。

## Registry v2

Registry使用`skiff-package-service-dev-registry-v2`。顶层只有：

```json
{
  "schemaVersion": "skiff-package-service-dev-registry-v2",
  "profile": "dev",
  "roots": [
    {
      "kind": "service",
      "root": "/absolute/project/root",
      "serviceId": "example.com/users"
    }
  ]
}
```

每个entry持久保存`kind`、规范化绝对`root`和可选`serviceId`。普通Package root的canonical kind为
`package`；同时具有`package.yml`和`service.yml`的Package具有service role，其canonical kind为
`service`，`serviceId`必填且必须等于entry写入时`service.yml`声明的canonical service ID。普通Package
entry不得保存`serviceId`。数组按`kind + root` canonical排序，同一root或service ID不得重复。

读取分成两个阶段：

1. **结构读取**只校验JSON、schema、entry字段、绝对路径、排序与唯一性，不访问root文件系统；
2. **live root验证**在sync/watch需要构建时确认root仍存在、kind仍一致，service root的当前ID仍与持久
   `serviceId`相同。

这个分离保证root已经被删除时，`registry remove`仍能读取和修改registry。`add`必须先完成live root验证；
`remove <root-or-service-id>`同时按规范化root和持久service ID匹配，只允许唯一命中。零命中、多个命中，
或root解释与service ID解释命中不同entry时都fail closed，不猜测用户目标。

Registry修改必须在目标文件同一目录创建临时文件，完整写入并`fsync`后原子`rename`；支持目录同步的平台
还要同步父目录。不得原地截断目标文件。写入失败保留原registry。

Canonical CLI是：

```text
skiff service dev registry add <service-dir>
skiff service dev registry list
skiff service dev registry remove <root-or-service-id>
```

Skiff尚未发布，旧`skiff dev registry`语法直接删除，不保留alias或兼容分派。

## Last-known-good 与动态 root

registry暂时不存在、JSON损坏、schema错误或任一live root验证失败时：

- 当前committed activation和`lastKnownGoodRegistry`保持不变；
- 本轮明确失败并进入重试，不能把错误输入解释为空registry；
- 不提交`lastSuccessfulFingerprint`；
- 后续registry或root修复后无需再编辑其它源码即可自动恢复。

watch首次启动且尚无last-known-good时，同样不能投影空assembly；它保持无本地期望状态并按失败退避重新
读取。显式移除最后一个合法entry则不同：这是一个结构合法的空root集合，必须生成canonical empty
`RuntimeAssembly`和同profile、零deployment分区的empty `RuntimeConfigSnapshot`，再通过普通CAS提交
新generation。empty activation负责撤下先前全部dev services，不是特殊的Router清理旁路。

Router activation freeze对zero-deployment epoch仍以exact registered sessions作为参与方
（此时session的capability binding为空，也照常参与），否则`stack init`种下的empty generation 0
无法通过普通CAS提交第一个真实assembly。

命令行显式root是本次进程的静态输入，与每轮重新读取的registry entries组成canonical union。只有最终
effective root集合为空时才生成empty pair。

## 启动同步与 Activation CAS

Managed watch启动后、第一次activation前必须先读取`GET /__router/health`。Router health必须公开一个
exact committed tuple：

```text
profile
generation
runtimeAssemblyRef
runtimeConfigSnapshotRef
```

watch校验Router profile与本次effective profile一致，并以health中的generation作为第一次
activation的`expectedGeneration`。不得在managed instance launcher或watch内部写死`0`。低层
`skiff assembly activate`仍保留显式expected generation，因为它是调用方直接操作CAS的接口。

每次期望状态按以下顺序收敛：

1. 构建或复用exact assembly；
2. 构建并安全发布exact config snapshot；
3. 以最近一次Router health generation提交activation CAS；
4. 只有Router确认exact pair committed后，才提交`lastSuccessfulFingerprint`。

HTTP 409不表示可以盲目递增本地generation。watch必须重读health：

- 若Router committed tuple已经精确等于本次目标assembly/snapshot，视为幂等成功；
- 若generation相对本次CAS的expected generation已经前进且目标tuple不同，采用新generation并在有界
  退避后重新提交；
- 若generation没有变化、profile不符、health缺字段或tuple无法精确判断，则本次失败，回到正常
  health/read重试，不能用原expected generation紧循环。

CAS始终保留；读取health只是获得当前比较基线，不允许改成无条件“latest wins”写入。

## 失败、替换与退避

第一次看到新fingerprint立即尝试。失败后按`1s, 2s, 4s, 8s, 16s, 30s`退避，随后保持最多`30s`；不因
build、snapshot、Router不可用、Runtime reject或CAS冲突的错误类型而永久停止。等待期间出现新fingerprint
时，新期望状态立即替换旧pending并立刻尝试，旧状态不继续排队。

一次成功的边界是build、snapshot publish与activation commit全部成功。任何阶段失败都不能推进
`lastSuccessfulFingerprint`。已经发布的immutable build/snapshot可以由后续重试安全复用，但复用不能
伪造activation成功。`--build-only`不属于managed activation成功边界，也不能推进后续managed watch使用
的activation fingerprint。

Watch退出或被替换时只停止本地循环，不修改Router committed state。Stable rollout、production release、
多operator仲裁和snapshot垃圾回收不在本文范围内。
