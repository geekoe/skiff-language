# Managed Dev Watch

本文只定义本地managed watch的registry、fingerprint、publish、withdraw与重试契约。Deployment release、
pointer和Runtime lazy-load语义由
[`runtime-lazy-load-deployment.md`](runtime-lazy-load-deployment.md)拥有；本文不定义artifact DTO、跨service
事务或Runtime load协议。

## 输入

Watch的期望状态来自：

- dev registry的schema、profile与entries；
- 命令行显式追加的静态root；
- effective roots的源码、control files、三层配置和Package解析输入；
- compiler/toolchain identity与dependency resolution结果。

Registry使用`skiff-package-service-dev-registry-v2`：

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

`root`必须是规范化绝对路径。普通Package使用`kind: package`且没有`serviceId`；同时拥有`package.yml`与
`service.yml`的root使用`kind: service`，其持久`serviceId`必须等于authoring声明。数组按`kind + root`
canonical排序；root或service ID不能重复。

读取分两步：结构读取只检查JSON/schema/排序/唯一性，不访问文件系统；live validation在build前确认root
存在、kind和service ID仍匹配。这样已删除目录仍可被`registry remove`唯一定位。Add必须先做live validation；
remove同时按规范化root与持久service ID匹配，零命中或歧义都fail closed。

Registry修改在同目录写临时文件，flush后原子rename；支持目录同步的平台还要同步父目录。旧
`skiff dev registry`拼写不保留，canonical CLI是：

```text
skiff service dev registry add <root>
skiff service dev registry list
skiff service dev registry remove <root-or-service-id>
```

## 状态与 fingerprint

Watch至少持有：

```text
lastKnownGoodRegistry
pendingFingerprint
lastSuccessfulFingerprint
ownedReleasePointers: key -> lastPublishedBuildId
```

Fingerprint覆盖effective profile、canonical root集合、每个root的完整build输入、dependency resolution和
toolchain identity；Router health、当前loaded set、release pointer mtime和上次错误不进入fingerprint。
Watch每轮poll与registry文件变化后都重新读取registry，不能只在启动时展开一次root集合。

配置输入只以canonical protected-store writer返回的store-domain keyed `BakedConfigPayloadRef`进入fingerprint；
fingerprint、ledger与日志都不得保存Secret明文或它的unkeyed digest。该ref的算法仍由artifact contract唯一拥有，
watch只消费结果。

`ownedReleasePointers`是本watch实例持久local ledger，只记录它成功发布过的service pointer。它不能扫描
整个profile后声称所有pointer归自己，也不能删除另一个operator创建的key。

## 收敛流程

一次fingerprint的plan按dependency顺序执行：

1. 编译或复用PackageArtifact、ServiceContract、ServiceDeployment与owned `BakedConfigPayload`；
2. 完整写入并验证全部immutable records；
3. 对每个service root原子更新
   `(profile, serviceId, exactVersion) -> deployment buildId`，同时更新owned ledger；
4. 对从effective roots移除且仍由ledger拥有的service，只有pointer target仍等于ledger记录的旧buildId时才
   原子unset；若已被外部更新则报告ownership conflict，不覆盖；
5. 可选请求Router/runtime按buildId做health/smoke验证；它不是发布原子性的一部分；
6. 全部planned publish/withdraw完成后才提交`lastSuccessfulFingerprint`。

普通Package root只发布供依赖解析的immutable Package records，不创建service release pointer。Service配置
变化通过canonical artifact writer生成新的deployment-owned `BakedConfigPayloadRef`与ServiceDeployment/buildId；
watch不另行定义config identity或publish入口。显式移除最后一个root只withdraw仍由ledger拥有的service pointer。

多service收敛由一组彼此独立的单键pointer update组成，读者可能在watch执行期间观察到新旧build混合；
Service boundary每次invocation仍按自己的pointer解析。Watch不得在这些pointer之上增加共同commit点。

## 失败与重试

Registry暂时缺失/损坏、live root无效、compile/publish/pointer/verify失败时：

- `lastKnownGoodRegistry`、已成功pointer和`lastSuccessfulFingerprint`不伪装成新状态；
- 合法的已完成单键publish无需回滚，重试按content identity与pointer target幂等复用；
- 失败输入不能被解释为空registry；修复文件后无需触碰其它源码即可恢复；
- 首次失败且没有last-known-good时不发布或withdraw任何pointer。

第一次看到新fingerprint立即尝试。失败后按`1s, 2s, 4s, 8s, 16s, 30s`退避，之后上限保持30s；等待时
出现新fingerprint会替换旧pending并立即尝试。Watch退出只停止本地循环，不修改release pointer。

Stable production rollout、多operator仲裁、artifact GC和跨service原子部署不属于managed watch。
