# P5-F298 Assembly service error type index

状态：Ready。

## 直接父节点与权威链

- linked/type-plan结果：
  `P5-F297-applied-nominal-linked-consumer-result.md`
- shared error model：
  `P5-F284-open-error-model-acceptance-result.md`
- runtime owner审计：
  `P5-F280-open-service-error-channel-implementation-audit-result.md`

父链继续引用唯一权威架构。启动时只读本任务；需要依据时沿父链向上读取。

## DAG位置与并行边界

- 节点：F280 W2-R中的assembly-owned双向`ServiceErrorTypeIndex`。
- 与F296 compiler、F299 local carrier并行；本任务禁止修改compiler、runtime model/eval/boundary。
- 输入：
  - PackageArtifact只持有`PackageSchemaIndexRef`；
  - canonical store已有完整index与records；
  - F297 linked image保留exact declaration/AppliedNominal facts。
- 完成后解除canonical service error export/import orchestrator。
- 当前是实现检查点，不是稳定候选。

## 唯一production范围

- `runtime/loader/**`
- `runtime/linked-program/**`中只新增index DTO/read-only linked image surface
- `runtime/linker/**`中只构建/验证index

允许co-located tests/fixtures。禁止修改runtime model/eval/boundary/request/host/transport、
artifact model/identity、compiler、router、std、生态仓库或权威文档。

## 完成标准

### 1. Exact index加载

- loader按每个PackageArtifact的exact `PackageSchemaIndexRef`从resolver读取完整index；
- 验证index identity、package owner、stable key、type id、public path/nameability与所有record refs；
- 加载并验证每个`PackageSchemaTypeRecord`的content identity与descriptor closure；
- 不再只加载operation contract scoped records，也不从contract `errors`收集roots；
- missing/conflicting index/record、owner/key/id不一致、duplicate stable key/public path全部fail closed。

### 2. Assembly-owned双向表

linked image中建立唯一immutable `ServiceErrorTypeIndex`，至少支持：

```text
execution TypeAddr / exact declaration-or-branch key
  -> owner packageId + stableSchemaKey + PackageSchemaTypeId
     + PublicNameable/SchemaClosed record/codec input

packageId + stableSchemaKey + PackageSchemaTypeId
  -> caller-linked execution address set + exact declaration/branch context
```

- 本任务只保存后续可物化为`CatchIdentity`的linked facts，不依赖或定义runtime-model的
  `CatchIdentity`；
- 由index publicPath与同PackageArtifact `implementation_links.types`解析execution type；
- owner始终是类型自己的Package，可以是throwing service的dependency；
- 同一exact public identity可映射多个等价execution address，不误报collision；
- 同一address映射多个public identity、同type id冲突record、public path无execution type、
  descriptor不一致全部fail closed；
- named union的enclosing/branch identity与representation owner保留，不按shape猜；
- generic PackageSchema/public error本轮fail closed。

### 3. 职责边界

- index只保存typed owner/record/link facts，不编码异常、不生成InternalError、不持有request stack；
- operation contract/schema closure不是error lookup owner；
- 不在throw时按public path/display/source path/short name/suffix/record shape扫描；
- 不硬编码`std.service.InternalError` schema identity；std作为普通Package从同一index解析；
- 不增加legacy artifact reader或fallback。

## 最小测试与验证owner

至少覆盖：

- service自身public error；
- dependency Package public error；
- 同identity多个execution address；
- representation与named-union branch；
- missing index/record/link、owner/key/id篡改、conflicting record、multi-identity address；
- applied PackageSchema/generic public fail closed；
- operation contract没有errors仍可从owner Package index建表；
- exact owner不同但public path/shape相同不合并。

唯一owner：

```bash
cargo test -p skiff-runtime-loader --lib -- --list
cargo test -p skiff-runtime-loader --lib --no-fail-fast
cargo test -p skiff-runtime-linked-program --lib -- --list
cargo test -p skiff-runtime-linked-program --lib --no-fail-fast
cargo test -p skiff-runtime-linker --lib -- --list
cargo test -p skiff-runtime-linker --lib --no-fail-fast
git diff --check
```

若其它runtime旧consumer在枚举前遮挡，建立最窄owner test target并记录精确首错；不得越界修复。
不运行workspace、runtime-eval、生态、stable、live或chat smoke。

## 风险与交付

- 风险：高；后续进入`A5-runtime-channel`独立验收。
- worktree：`/Users/geek/workspace/skiff-p5-f298-service-error-index`
- branch：`codex/p5-f298-service-error-index`
- 不push、不操作stable。
- 启动到第一次production修改不超过5分钟；不可执行时返回
  `TASK_NOT_EXECUTABLE`、精确缺口与最小前置。
- 提交后返回commit、index shape、fail-closed矩阵、自验收与遮挡；不承接codec/eval/wire。
