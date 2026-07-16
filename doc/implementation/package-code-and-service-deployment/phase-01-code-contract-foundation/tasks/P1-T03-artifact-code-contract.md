# P1-T03：定义 Package Code / Service Protocol Artifact 契约

状态：`ready`
类型：Artifact model / identity
依赖：P1-T00、P1-T01、P1-T02
执行者：Artifact Contract Agent，一份提交

## 目标

在共享artifact crate中定义Phase 01所需的typed code contract和T00的
`ServiceProtocolContract` view，并把它们纳入唯一identity实现。此任务只建立schema、构造器、
校验和identity，不做compiler analysis/projection。

## 产出模型

实际命名可做小幅调整，但职责必须一一对应：

```text
PackageUnit
  service_requirements: Vec<ServiceContractRequirement>
  callable_contracts: Vec<CodeCallableContract>
  boundary_abi_identity: String

CodeCallableContract
  callable_id / local_code_abi_ref
  effect_summary
  link_requirements
  boundary_projection: Available(BoundaryOperationContract)
                     | Unavailable(Vec<BoundaryUnavailableCode>)
```

相关 value/callback/stream/error plan 必须是 typed enum/struct，不能用 `MetadataValue`、raw JSON、
字符串 tag 或缺字段表达状态。

现有 `PublicationAbiUnit`/`PackageExportIndex` 继续拥有 canonical Local Code ABI signature。
`CodeCallableContract` 必须以稳定 id/ref关联它，不能嵌入第二份独立生成的local signature；若为读取
方便提供view，也必须由同一个canonical DTO派生且不参与双向一致性维护。

## Identity 规则

- package build identity：包含完整 executable/code contract 和 service requirements。
- package ABI identity：表示 Local Code ABI surface；是否包含内部实现仍遵守现有规则。
- boundary ABI identity：包含 public callable 的稳定 availability code 或完整 boundary contract。
- diagnostic detail、source span、artifact root/path、provider build id 不进入 ABI identity。
- `Unavailable` reason 使用稳定 code；排序和集合 canonicalization 明确且可测试。
- identity 只能在 `skiff-artifact-identity` 计算；compiler wrapper 不实现 hash。
- service protocol identity由同一crate对canonical named operation surface计算；deployment字段不
  进入其identity。

## Service requirement 规则

`ServiceContractRequirement`按T00表达：alias、service id、精确contract version、protocol
identity和实际引用operation的typed expectation。`ServiceProtocolContract`是具名operation map；
service/deployment revision与package build不进入其identity。requirement不包含provider package
id；provider build id若编译时用于证明读到的artifact完整性，也不得写入这个可寻址contract。

`ServiceUnit`必须暴露一个严格typed的 `ServiceProtocolContract` subobject/view作为compiler-input
唯一读取源，替换“分别读取protocol identity、PublicationAbi、operations再自行拼合”的做法。
Phase 01旧service-source emitter由T10填充它；Phase 02收窄ServiceUnit时继续保留该contract。

## 范围与 ownership

主要路径：

- `artifact-model/src/`：新增按职责拆分的 code/effect/link/boundary 模块
- `artifact-model/src/package_unit.rs`：只做聚合，不把所有新类型堆入该文件
- `artifact-model/src/service_unit.rs`：接入单一typed protocol contract view，不复制operation DTO
- `artifact-model/src/lib.rs`、schema version 与 schema tests
- `artifact-identity/src/package.rs`、`service.rs` 或 T02 创建的对应 owner
- 必要的 compiler typed re-export/wrapper，只允许薄适配

## 非目标

- 不解析 `package.yml`。
- 不计算 effect/link facts。
- 不生成 boundary projection。
- 不更新 package compiler production path；T10 负责。
- 不兼容旧 PackageUnit JSON。

## 实现约束

- schema 变化必须 bump 相应 version；缺新字段 fail closed。
- `PackageUnit::empty` 等测试 builder 必须给出显式、语义正确的 empty contract；production
  deserialization 不得 `serde(default)` 接受旧 artifact。
- 不继续扩大 `artifact-identity/src/lib.rs`，使用 T02 拆出的 package domain。
- 如果现有 `ConfigAndEffectMetadata` 还承载 config，可把 config 与 typed effect 分开；不得为了
  一次提交方便保留两个 effect source of truth。

## 必须测试

- 新类型 serde round-trip、unknown field rejection、missing field rejection。
- `Available` 与 `Unavailable` 的互斥和稳定排序。
- build/local ABI/boundary ABI identity 的正交变化矩阵。
- ServiceProtocolContract具名operation排序、protocol identity和deployment字段排除矩阵。
- diagnostic wording/path/build-id 变化不影响 boundary identity。
- compiler identity wrapper 与 artifact-identity 结果一致。

## 聚焦验证

```bash
cargo test --no-fail-fast -p skiff-artifact-model -p skiff-artifact-identity
node scripts/check-artifact-identity-single-source.mjs
git diff --check
```

## 验收标准

- 下游 crate 可以不看 AST/JSON，仅从 PackageUnit 判断 callable 的 local/boundary contract。
- Local Code ABI signature只有PublicationAbi/export index一个owner，callable facts只做typed索引。
- artifact 中不存在“effect 尚未分析但看起来是 Empty”的状态。
- service requirement 不以 provider build/package identity 寻址。
- ServiceUnit的compiler contract view不再由多个字段临时拼装。
- 新生产文件职责单一；没有新增超长聚合文件。

## 停止条件

- T00未能确定service contract surface/version/revision边界；
- T01 未能确定 linkable/recoverable/callback plan 的层次；
- 一个字段必须同时由 artifact-model 与 compiler 自己定义；
- identity 需要 diagnostic 文本或 provider deployment identity；
- 需要兼容旧 schema 才能使测试通过。

## 提交

提交信息建议：`feat(artifact): add package code and boundary contracts`
