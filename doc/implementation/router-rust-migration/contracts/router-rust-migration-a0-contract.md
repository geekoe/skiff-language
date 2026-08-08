# Router Rust Migration A0 Contract — Actor Routing Projection

日期：2026-08-02
状态：frozen（A0 交付物；消费者 A1/A2/A3 在合入前不得编码消费）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md` §2.4（actor routing
  projection contract）、§3.2（stateless `ActorMethodCatalogView` / actor owners）、
  §3.3（immutable `RoutingEpoch` 内 actor index）、§3.4（identity/fence 类型）、
  §5.4（C-actor pack 前置）、§7 E-actor-rust / E-actor-parity。
- 直接父批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-3.md`（A0 条款与验证
  owner）。
- 叶子任务：`doc/implementation/router-rust-migration/execution/router-rust-migration-a0-leaf.md`。
- 架构语义事实源：`doc/architecture/actor-model.md`（Identity 与注册、任期与 Version、
  边界规则）。

冲突时以权威设计为准；本契约不修改设计语义，只冻结 §2.4 委托 A0 定义的最小投影。

## 1. 目的与冻结范围

冻结 actor routing projection 的 schema、owner 与 identity generation，使
compiler/deployment producer（A1）、TS Router strict consumer（A2）、Rust strict
reader/consumer（A3）可以并行编码且互不猜测：

1. stable actor ref；
2. method admission / implementation identity；
3. exact deployment binding；
4. 明确排除 source、File IR 与 executable payload。

本契约冻结的是**投影数据契约**，不是 wire frame。wire frame 映射由 C-model-actor
pack 定义；activation DTO 由 contracts-activation 定义；bootstrap / artifact refs
由 contracts-bootstrap 定义。

## 2. 冻结的 schema

canonical 类型 owner：`skiff-deployment` crate 的 `projection::actor_routing` 模块。
serde 形态固定为 `camelCase` + `deny_unknown_fields`；schemaVersion 固定为
`skiff-actor-routing-projection-v1`。

```rust
pub struct ActorRoutingProjection {
    pub schema_version: String,              // "skiff-actor-routing-projection-v1"
    pub methods: Vec<ActorRoutingMethod>,    // 按完整 typed key 排序，唯一
}

pub struct ActorRoutingMethod {
    pub actor: ActorRoutingRef,              // stable actor ref
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub method_identity: ActorMethodIdentity,
    pub deployment: ServiceDeploymentRef,    // exact deployment binding
    pub package: PackageArtifactRef,         // owning package binding
}

pub struct ActorRoutingRef {
    pub service_id: String,                  // actor 的 home service
    pub actor_abi_identity: ActorAbiIdentity,
}
```

JSON 形态示例：

```json
{
  "schemaVersion": "skiff-actor-routing-projection-v1",
  "methods": [
    {
      "actor": {
        "serviceId": "example.com/docs",
        "actorAbiIdentity": "skiff-actor-abi-v1:sha256:<hex64>"
      },
      "actorImplementationIdentity": "skiff-actor-implementation-v1:sha256:<hex64>",
      "methodIdentity": "skiff-actor-method-v1:sha256:<hex64>",
      "deployment": {
        "serviceId": "example.com/docs",
        "contractVersion": "1.0.0",
        "deploymentRevision": "<revision>",
        "deploymentArtifactIdentity": "skiff-deployment-artifact-v4:sha256:<hex64>"
      },
      "package": {
        "packageId": "example.com/docs-package",
        "packageVersion": "1.0.0",
        "packageBuildId": "skiff-package-build-v10:sha256:<hex64>",
        "packageLocalAbiIdentity": "skiff-package-local-abi-v7:sha256:<hex64>"
      }
    }
  ]
}
```

构造不变式（构造时校验，失败即拒绝整个投影）：

- `schema_version` 必须精确等于 `skiff-actor-routing-projection-v1`；
- 所有 identity 字段必须是 framed `prefix:sha256:<hex64>` 且前缀精确匹配
  `skiff-actor-abi-v1:sha256` / `skiff-actor-implementation-v1:sha256` /
  `skiff-actor-method-v1:sha256`；deployment 与 package identity 同样校验其
  `skiff-deployment-artifact-v4:sha256` / `skiff-package-build-v10:sha256` /
  `skiff-package-local-abi-v7:sha256` 前缀；
- `actor.service_id` 必须等于 `deployment.service_id`（actor ref 与 binding 不脱离）；
- entries 按完整 typed key 排序；重复 entry 拒绝；
- 空 `methods` 合法（无 actor 的 assembly）。

## 3. Owner 与依赖方向

| 事实 | Owner |
| --- | --- |
| 投影 DTO / 构造校验 | `skiff-deployment::projection::actor_routing`（A0 冻结） |
| identity generation | `skiff-artifact-identity::actor`（`actor_abi_identity` / `actor_method_identity` / `actor_implementation_identity`，已有 canonical 实现，A0 引用不迁移） |
| identity newtype | `skiff-artifact-model::actor_declaration`（`ActorAbiIdentity` / `ActorImplementationIdentity` / `ActorMethodIdentity`，已有，不迁移） |
| deployment / package ref | `skiff-artifact-model`（`ServiceDeploymentRef` / `PackageArtifactRef`，已有） |
| actor method catalog view | 消费者（A2/A3）；只读投影，不建立独立 index / refresh（§3.2 / §3.3） |

依赖方向：`skiff-deployment` → `skiff-artifact-model` + `skiff-artifact-identity`。
Router（TS/Rust）只能作为投影 consumer，不得反向定义或复制投影 schema。

## 4. Identity generation 规则（冻结）

identity generation 的 canonical 实现与语义已经存在于 `skiff-artifact-identity`，
本契约固化其消费规则，不新增第二套生成器：

1. `actor_abi_identity(ActorAbiInput)`：preimage 为 `{schema, abi}`，输出
   `skiff-actor-abi-v1:sha256:<hex64>`。覆盖 actor 名称、key 字段类型与 canonical
   编码、字段布局、公开成员方法签名与 actor runtime ABI（actor-model.md "任期与
   Version"）。service version / build id 不进入。
2. `actor_method_identity(module_path, actor_name, method_name)`：preimage 为
   `{schema, module_path, actor_name, method_name}`，输出
   `skiff-actor-method-v1:sha256:<hex64>`。签名事实保留在 ABI identity 中，方法
   更名即新 method identity。
3. `actor_implementation_identity(units, actor_module_path, actor_name)`：preimage
   为 `{schema, actor_abi_identity, roots, executables, constants, types}`，输出
   `skiff-actor-implementation-v1:sha256:<hex64>`。覆盖规范化可执行 IR 的
   reachable SCC（含 create 根），索引与无关代码规范化后不影响 identity。

**投影只携带生成后的 framed identity 字符串，绝不携带生成输入**（module_path /
actor_name / method_name / 可执行图 / type table / sourceSpan）。

stable actor ref 的 identity 规则：

- ref = `service_id` + `actor_abi_identity`；
- `actor_abi_identity` 已经 canonical 覆盖 actor 类型、key 字段类型与 key canonical
  编码，因此不新增独立的 `ActorTypeIdentity` hash，也不在投影中重复携带
  `actorIdTypeIdentity` / `actorIdEncodingVersion`（见 §7 决策记录 D1/D3）。

## 5. Exact deployment binding（冻结）

一个 entry 必须绑定：

- `ServiceDeploymentRef`：`serviceId` / `contractVersion` / `deploymentRevision` /
  `deploymentArtifactIdentity` —— 与 Runtime session 注册的 exact tuple 一致；
- `PackageArtifactRef`：`packageId` / `packageVersion` / `packageBuildId` /
  `packageLocalAbiIdentity` —— actor 声明所在的不可变 package artifact。

package binding 是必需的精确性事实：(abi, implementation, method) 三元组在同一
service 的不同 package 间可能相同（module path 是包内命名空间，两个包可以各自声明
相同 module/actor 形状），因此必须用 `packageBuildId` 消除声明归属歧义。

不变式：`actor.service_id == deployment.service_id`。

## 6. 反例：不得进入投影

| 禁止内容 | 反例字段/形态 | 理由 |
| --- | --- | --- |
| source | 源码文本、`sourceSpan`、`sourceAstHash` | 违反 §2.4 最小投影 |
| File IR | `modulePath`、`actorName`、`methodName`（作为字段）、`unit`、`file`、`fileIrIdentity`、`loadedFileIndex`、`codeSlot` | File IR 坐标是加载期内部事实；identity 生成输入不得携带 |
| executable payload | `executableIndex`、可执行体、常量/类型表、payload bytes | 违反 §2.4；implementation identity 已覆盖实现图 |
| symbol path | `actorTypeIdentity`（符号路径串）、`actorSymbol` | 显示/链接串，非 canonical identity；runtime 可自行从声明派生 |
| 其它 | 任何额外字段 | serde `deny_unknown_fields` 在边界拒绝 |

所有上述内容都被类型结构（无对应字段）与 serde 边界（unknown field 拒绝）双重排除。

## 7. 冻结决策记录

| ID | 决策 | 选择与理由 |
| --- | --- | --- |
| D1 | stable actor ref 形态 | `{service_id, actor_abi_identity}`。actor-model.md 规定 service id 属于 actor identity；ABI identity 已覆盖 actor 类型、key 类型与编码，ref 不需要更多字段。备选：把 `actorTypeIdentity` 符号路径放入 ref —— 拒绝，符号路径是派生链接事实且可能跨包歧义，且违反"不含 source"红线。 |
| D2 | exact binding 含 owning package | 除 `ServiceDeploymentRef` 外携带 `PackageArtifactRef`，消除跨包 (abi, implementation, method) 歧义（§5）。备选：只带 deployment ref —— 拒绝，无法精确定位声明 owner。 |
| D3 | 不新建 `ActorTypeIdentity` hash | 新建 hash 会与 ABI identity 重复持有同一事实并改变公共契约；wire 侧如需要符号路径仍由 C-model-actor 决定，不属于投影。 |
| D4 | schema version 独立字符串 | `skiff-actor-routing-projection-v1`；schema 变更必须先改本契约与版本。 |
| D5 | entry 排序/唯一性 | 按完整 typed key 排序 + 重复拒绝，构造与输入顺序无关，满足 immutable epoch 一次构造语义。 |
| D6 | 构造期 identity 前缀校验 | 投影是公共契约边界，构造/反序列化即校验 framed 形态，fail closed。 |

## 8. 非目标与边界

- 不定义 wire frame（C-model-actor 负责：wire 是否继续携带 `actorTypeIdentity` /
  `actorIdTypeIdentity` / `actorIdEncodingVersion` / owner 字段由该 pack 决定）。
- 不实现 A1 producer / A2 TS consumer / A3 Rust reader。
- 不改 router production 代码（TS 或 Rust）。
- 不做 activation DTO（contracts-activation）。
- 不定义 bootstrap / artifact refs（contracts-bootstrap）。
- 不迁移或修改 `skiff-artifact-model` 现有 identity 类型与
  `skiff-artifact-identity` 生成器。

## 9. 验证与反向搜索

- canonical 类型编译/测试：`cargo check --manifest-path deployment/Cargo.toml`、
  `cargo test --manifest-path deployment/Cargo.toml actor_routing --no-fail-fast`
  （10 项：排序确定性、duplicate 拒绝、schema version、三个 identity 前缀、
  serviceId 一致、空投影、serde 精确表面、File IR/payload 字段拒绝）。
- 反向搜索：本契约冻结前，`rg` 证明 `actor_routing` / `ActorRouting*` /
  `ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION` 只出现于
  `deployment/src/projection/actor_routing*`、`projection/mod.rs` 与本契约/叶子
  文档；router TS/Rust、compiler、runtime、contracts-* 均无提前 consumer。
- 消费者合入冻结后必须只读本投影，不得回退读取 PackageArtifact/File IR。
