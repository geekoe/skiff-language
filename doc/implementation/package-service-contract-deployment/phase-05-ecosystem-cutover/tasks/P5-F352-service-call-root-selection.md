# P5-F352 Explicit service-call root selection

状态：Ready（C1 shared Package/Service checkpoint）。

## 直接父节点

- `P5-H36-external-ingress-implementation-dag.md`
- `P5-F349-public-generic-boundary-availability-audit-result.md`

F351已在integration实现HTTP gateway shared model，但本任务不消费gateway类型。权威语义只沿父节点引用链
读取；不得重新定义`api.yml`或ServiceContract。

## DAG位置与目标

把`api.yml serviceCall: true`从authoring一直保真到PackageArtifact，并使ServiceContract只投影显式roots。
完成后解除deployment HTTP model与后续compiler convergence；本任务不实现external ingress。

必须完成：

1. 唯一canonical `api.yml` parser：
   - `api.yml`必须存在、非空且root是mapping；无公开面写`{}`；
   - 普通function leaf仍可写string；
   - object function leaf严格为`source + serviceCall: true`，拒绝false、缺字段和unknown field；
   - public instance leaf允许严格`serviceCall: true`，并保留`const + interfaces`；
   - marker只能用于function/public instance，且只允许service root；
   - 删除`compiler/source`中的重复parser/规则，复用一个owner，不保留双读。
2. PackageArtifact保存typed显式roots：
   - function root精确绑定public path与`PackageCallableId`；
   - instance root展开其显式listed interface methods并精确绑定method callable ids；
   - roots进入Package build identity与strict artifact validation，但不进入PackageLocalAbiIdentity；
   - bump PackageArtifact schema/build-identity generation；无default、旧reader或兼容fallback。
3. Service projection：
   - 只投影显式roots；未标记Available/Unavailable callable都只属于Package API；
   - 标记root若Unavailable，一次报告其全部结构化原因，不能静默排除；
   - public instance root只生成listed interface methods；
   - 零marker生成合法零operation ServiceContract及稳定identity；
   - operation/schema closure只从选中Available roots计算；
   - Package API visibility仍展示全部public callable，但只有选中root带service operation id。
4. 所有受PackageArtifact字段/generation影响的repo内构造器、strict wire tests与golden同步更新。

## 关键production入口与风险

- authoring：`compiler/input/src/api_yml.rs`、重复的`compiler/source/src/api_yml.rs`；
- typed API graph：`compiler/input-model`、`compiler/source/src/api*.rs`、
  `compiler/projection-input`；
- artifact：`artifact-model/src/package_artifact.rs`、`artifact-identity/src/package_artifact/**`；
- projection：`compiler/projection/src/package_artifact/**`；
- contract：`compiler/contract/src/projection.rs`及compile validation；
- service-only marker admission：`compiler/driver` package/service pipeline。

最高风险是marker在source与artifact之间丢失、marker污染Local ABI identity、零operation仍被旧validation
拒绝、或public instance methods被按普通function重复/漏投影。

## 写入范围

允许修改上述production owner、其Cargo依赖/exports、直接tests/fixtures/golden，以及因
PackageArtifact严格新增字段导致无法编译的机械构造器。

禁止修改：

- `artifact-model/src/gateway.rs`、`artifact-identity/src/gateway.rs`；
- `ServiceManifestAuthoring.http`、deployment gateway/ingress DTO；
- generic PackageSchema eligibility规则（F353 owner）；
- runtime、router、test-runner、三仓库service、stable/live配置、lockfile。

若发现必须改变公共语义或与F353写入同一production文件，立即报告，不自行吞并。

## 验证

先列出并确认非零selector，再运行聚焦测试：

```bash
cargo test -p skiff-compiler-input api_yml -- --list
cargo test -p skiff-compiler-contract service_call -- --list
cargo test -p skiff-artifact-identity package_artifact -- --list
cargo test -p skiff-compiler-input api_yml
cargo test -p skiff-compiler-contract
cargo test -p skiff-artifact-model package_artifact
cargo test -p skiff-artifact-identity package_artifact
cargo test -p skiff-compiler service_call
cargo fmt -p skiff-artifact-model -p skiff-artifact-identity -p skiff-compiler-input-model \
  -p skiff-compiler-input -p skiff-compiler-source -p skiff-compiler-projection \
  -p skiff-compiler-contract -p skiff-compiler -- --check
git diff --check
```

必须有正/负证据覆盖：普通package marker拒绝、missing/empty api.yml拒绝、`{}`成功、string unmarked、
object marked、false/unknown拒绝、marked unavailable全原因、unmarked available不进contract、
零operation、public instance全listed methods、root重排identity稳定、root变化只改变build/contract而不改
Local ABI identity、旧PackageArtifact wire拒绝。

不运行workspace/root、stable/live，不push。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f352-service-call-selection`
- branch：`codex/p5-f352-service-call-selection`
- 从包含本task的integration checkpoint创建；result记录exact base/commit/tree。
- 提交production/tests，再提交result；worktree保持clean，不merge/rebase integration。
