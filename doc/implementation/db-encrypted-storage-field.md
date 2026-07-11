# DB Object Encrypted Storage Field 实现文档

状态：implemented
日期：2026-07-10

## 0. 结论

本阶段只为 Skiff `db object` 实现顶层字符串字段的透明存储加密。`type` 仍然只描述内存数据，不引入 `encrypted<T>`、secret type 或加密 annotation；加密是 DB attachment 上的持久化映射：

```skiff
type ProviderCredential {
  id: string,
  ownerUserId: string,
  providerId: string,
  apiKey: string,
}

db object ProviderCredential {
  primary key(id)
  storage apiKey using encrypted
  unique index byOwnerProvider(ownerUserId asc, providerId asc)
}
```

业务内存里的 `apiKey` 始终是普通 `string`。service-db 在写 Mongo 前将它转换为认证密文，在从 Mongo 读取后还原为普通字符串。应用仍负责不把内存字符串写入日志或 API；本功能只提供 DB、数据库备份和离线存储层的 confidentiality/integrity。

V1 使用 runtime 私有 keyring 文件、HKDF-SHA256 派生字段 key、AES-256-GCM 和随机 nonce。不接 KMS，不实现通用用户自定义 codec，不实现 derived/non-stored 字段，也不重构现有 Date storage projection。

## 1. 目标与非目标

### 1.1 目标

1. 用户可在 `db object` 中声明一个顶层 `string` 字段使用 `encrypted` storage mapping。
2. insert、replace、upsert、按 primary key update 和 read 对业务代码透明；Mongo 永远不出现该字段的明文。
3. 同一明文重复写入生成不同密文；密文、nonce、AAD、record key、service/collection/field 任一不匹配都必须 fail closed。
4. encrypted 字段不能用于 predicate、regex、order、index 或局部修改；compiler 与 runtime 双重拒绝。
5. encryption root key 只由 runtime host 读取，不进入 Skiff source、service config、artifact、router control frame、Mongo、日志或错误消息。
6. keyring 支持 active key 写入和旧 key 读取，为后续轮换保留安全路径。
7. source、artifact、package projection、linker 和 runtime metadata 对 storage mapping 使用同一个显式契约。

### 1.2 非目标

- 不实现 `encrypted<T>` 或修改任何普通 `type` 的语义。
- 不实现 nullable string、number、Date、record、array、Json、map、union、recoverable envelope 或 immutable file 的加密。
- 不实现嵌套 field path 加密；只允许 attached type 的顶层字段。
- 不实现 deterministic/searchable encryption，也不允许按密文字段做 equality query。
- 不实现 KMS/Vault、远程 key provider 或 per-service key provisioning API。
- 不实现用户自定义 storage codec、codec 组合、字段重命名、压缩或日期格式声明。
- 不实现 derived/non-stored/generated 字段。
- 不实现现有明文字段的原地自动迁移，也不做 plaintext fallback/dual read。
- 不实现自动轮换 CLI。V1 支持多 key 读取和 active key 写入；退役旧 key 由服务自己的显式数据迁移完成。
- 不实现内存 taint、secret redaction type 或禁止 `std.json.encode`。读取后的值是普通字符串。
- 不防止同一 record、同一 field、同一 key id 下的历史合法 envelope 被重放。AEAD 没有可信外部单调状态；把 revision 和密文一起放在 Mongo 中也不能防数据库攻击者同时回滚两者。
- 不面向高频写字段。V1 用于 API Key 等低频 secret；单个 derived field key 达到 `2^32` 次写入前必须轮换 root key，runtime 不维护持久写次数计数。
- 不在本阶段改 Agine/AIHub；BROK 在本能力合并后另行接入。

## 2. 当前状态与关键约束

### 2.1 当前编译链

`type` 与 `db object` 已经分层：同名 `db object` attachment 只保存 collection、primary key、retention、lease 和 index。所有 attached type 字段当前都按 identity/类型投影持久化。

关键路径：

```text
syntax AST DbDecl
  -> compiler/source DbAttachment + PublicationDbMetadata
  -> compiler/lowering DbMetadataIr
  -> artifact-model DbObjectFieldIr
  -> compiler/projection package/service DB metadata
  -> runtime linker DbObjectFieldIr
  -> runtime/service-db DbFieldMetadata
  -> mapping.rs BSON encode/decode/query/update
```

`artifact-model::DbObjectFieldIr` 和 linked-program 同名结构目前只有 `name + type`。本阶段增加 storage metadata，并保证 service-owned DB 与 package DB projection 都保留它。

### 2.2 当前 storage projection

`runtime/service-db/src/mapping.rs` 已经在内存/wire JSON 与 BSON 之间处理：

- `Date` RFC3339 string ↔ BSON Date；
- record/array 递归；
- recoverable envelope 整字段封装；
- `ImmutableFile` 生命周期相关路径。

加密必须进入同一 BSON mapping 边界，不能在 eval、业务 package 或 Mongo adapter 旁路再造第二套文档编码。

### 2.3 双写入/读取路径

service-db 同时有：

- `DbDocument`/serde JSON 路径，用于 capability 和低层测试；
- `RuntimeValue + RequestHeap` 路径，用于生产 evaluator，且附带 recoverable context。

如果分别实现加密，它们很容易在 AAD、envelope、错误或操作限制上漂移。因此本阶段必须先建立共享的 field storage encode/decode 和 use-policy helper，两条路径都只能调用这一个入口。

### 2.4 更新时的 record key

V1 AAD 绑定 Mongo `_id`，因此加密前必须知道目标 record key：

- insert/insert many：body 中有 key；
- replace by key：selector 提供 key；
- replace by query：完整 replacement body 按现有规则包含 key；
- upsert by key：selector 提供 key；
- update by key：selector 提供 key；
- update by query/update many：编码 `$set` 时不知道实际 `_id`，V1 禁止设置 encrypted 字段。

不能为了保留 update-many 便利而从 AAD 中去掉 record key，也不能给多行复用同一个 ciphertext/nonce。

为让 insert、selector、stored `_id` 和 read 使用唯一表示，任何声明 encrypted field 的 DB object，其 primary key V1 也必须是精确的非 nullable `string`（alias 最终展开为 `string` 可以接受）。AAD 使用该逻辑 string 的 UTF-8 bytes，不依赖 BSON 数值宽度或 library-specific standalone BSON encoding。

## 3. 用户可见语法与语义

### 3.1 Canonical syntax

```skiff
db object ProviderCredential {
  name "provider_credential"
  primary key(id)
  storage apiKey using encrypted
}
```

语法规则：

- `storage`、`using`、`encrypted` 只在 `db object` declaration 中是 contextual keyword。
- V1 `storage` 后只接受单个顶层 identifier，不接受 `profile.secret`。
- V1 `using` 后只接受 `encrypted`。
- 行尾分号沿用其他 DB declaration entry 的可选规则。
- 未声明 storage mapping 的字段保持当前 identity/类型投影行为。
- 同一字段重复声明 storage mapping 是 compile error。

### 3.2 字段约束

`storage field using encrypted` 必须满足：

1. field 存在于 attached record type；
2. field 不是 primary key；
3. field 类型精确为非 nullable `string`；alias 最终展开为 `string` 可以接受；
4. DB object 的 primary key 也必须是精确的非 nullable `string`；
5. field 的 DB boundary plan 必须是普通 scalar string，不能同时进入 recoverable envelope 或 immutable-file lane；
6. field 不出现在普通/unique index 的 fields；
7. field 不出现在 partial index `where`；
8. 一个字段最多一个 storage mapping。

错误必须指明 DB object 和字段，例如：

```text
db object ProviderCredential encrypted storage field `apiKey` must be a non-null string
```

### 3.3 Operation matrix

| Operation | encrypted field | 规则 |
| --- | --- | --- |
| `db insert` / `insert many` | 写 | 允许；每行独立 nonce/AAD |
| `db find/optional/require` by key | 读 | 允许，返回普通 string |
| query read 按其他字段筛选 | 读 | 允许 |
| `fields { apiKey }` | projection | 允许选择完整顶层字段并解密；沿用现有规则，primary key 自动包含在逻辑结果中 |
| `where apiKey ...` / `regex(apiKey, ...)` | predicate | compile/runtime 拒绝 |
| `order apiKey` | order | compile/runtime 拒绝 |
| index/partial index 引用 `apiKey` | index | publication/runtime metadata 拒绝 |
| `db update Target(key) { apiKey = value }` | set | 允许，selector key 进入 AAD |
| update by query / update many set `apiKey` | set | 拒绝，编码时没有稳定 record key |
| `+=/-=`、`add/remove`、nested set | partial change | 拒绝 |
| replace by key/query | full write | 允许；replacement 必须能解析 key |
| upsert by key insert/set | write | 允许；使用 selector key |
| delete/count/exists | 无直接字段使用 | 按现有语义 |

所有限制都在 compiler 提供早期错误，并在 runtime metadata/mapping 再验证一次，防止手工或旧工具构造非法 artifact。

projection 的既有语言契约是 primary key 总会自动包含。因此 `fields { apiKey }` 的逻辑结果是 `{ id: string, apiKey: string }`；物理 Mongo projection 必须包含 `_id`，既用于 materialize `id`，也用于 encrypted AAD。实现不能出现“逻辑结果不含 key、但私下为解密读取 key”的第二套 projection 语义。

### 3.4 内存与 API 语义

读取结果的 nominal/anonymous record 类型不变化：

```skiff
const credential = db require ProviderCredential(id)
// credential.apiKey: string
```

以下行为仍然合法，且由应用承担风险：

```skiff
std.json.encode(credential.apiKey)
```

DB encryption 不能声称保护已经进入 service memory 的值。service-db 自己不得把 plaintext 放入 tracing、panic、error display 或 assertion diff；业务代码的 redaction 是独立职责。

## 4. Source、Semantic 与 Artifact 契约

### 4.1 Syntax AST

在 `syntax/src/ast.rs` 增加：

```rust
pub struct DbStorageDecl {
    pub field: String,
    pub codec: DbStorageCodec,
}

pub enum DbStorageCodec {
    Encrypted,
}
```

`DbDecl` 增加 `storage: Vec<DbStorageDecl>`。parser 在 `parse_db_decl` 中识别：

```text
storage <ident> using encrypted
```

parser tests 覆盖 canonical syntax、可选分号、未知 codec、缺 `using`、嵌套 path 和重复语法形态；字段存在性/type/index 约束属于 semantic，不在 parser 猜。

### 4.2 Attachment 与 publication metadata

`DbAttachment`/`PublicationDbMetadata` 增加按字段名索引的 storage map。source semantic 一次性验证 §3.2 全部约束，后续 lowering 不重复解析 AST 文本。

package DB object 的 storage declaration 是包持久化契约的一部分。package projection 改 collection name 时只改物理 collection；encrypted 标记必须原样保留。

### 4.3 File IR / artifact model

增加可扩展但 V1 只有一个非 identity 分支的 artifact enum：

```rust
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DbFieldStorageIr {
    #[default]
    Identity,
    Encrypted,
}

pub struct DbObjectFieldIr {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeRefIr,
    #[serde(default, skip_serializing_if = "DbFieldStorageIr::is_identity")]
    pub storage: DbFieldStorageIr,
}
```

identity 缺省不是历史兼容层，而是绝大多数字段的 canonical compact representation。encrypted 必须显式写入 File IR、service DB metadata、package artifact、linked program 和 artifact identity。

需要更新并测试的复制边界：

- compiler lowering `DbMetadataIr`；
- compiler projection 的 service/package storage metadata；
- artifact-model JSON；
- runtime linker file conversion；
- linked-program DTO；
- package-test artifact builder/fixtures；
- compiler/runtime synthetic fixture constructors。

任何 mapping helper 都不能重建或默认丢弃 `storage`；增加一条 artifact round-trip + package projection test，证明 `encrypted` 从 source 到 runtime metadata 不丢失。

### 4.4 Compiler operation validation

`DbMetadataIr` 保存 `field_storage`，并提供统一 helper：

```text
storage_for_top_level_field(name)
validate_storage_field_use(path, use_case, selector_kind)
```

query lowering、order lowering、index validation 和 change validation 调用同一 policy。不能在每个分支复制 `if encrypted`。

`DbFieldUse` 至少区分：

```text
Projection | Predicate | Order | Index | WholeSet | PartialChange
```

其中 encrypted 只允许 top-level `Projection`，以及已知 record key 的 top-level `WholeSet`。insert/replace 走完整文档验证，不属于 query use。

source semantic 同时验证 DB primary key 是非 nullable string。runtime metadata 不信任 artifact：`Encrypted` 只有在 field plan 是 plain scalar string、key plan 是 plain scalar string，且 field 不属于 recoverable/immutable-file path 时才接受；否则 service activation 失败。这样手工 artifact 不能把 `Encrypted` 与 recoverable envelope 叠加后拖到写入时才报错。

## 5. 密文格式与加密上下文

### 5.1 算法

V1 固定：

- root key：keyring 中的 32-byte 随机值；
- field key：HKDF-SHA256 派生 32 bytes；
- AEAD：AES-256-GCM；
- nonce：每次写入由 OS CSPRNG 生成 12 bytes；
- authentication tag：使用 AEAD 输出，和 ciphertext 一起存储；
- plaintext：原始 UTF-8 string bytes；
- 不允许调用方选择 algorithm、nonce 或 key id。

派生参数：

```text
salt = UTF8("skiff-service-db-encrypted-field-v1")
info = tuple(
  "skiff-service-db-encrypted-field-hkdf-v1",
  keyId,
  storageServiceId,
  finalPhysicalCollectionName,
  topLevelFieldName
)
```

每个 root key id、storage service、collection 和 field 得到独立 field key。

### 5.2 AAD

AAD 使用确定性二进制编码：

```text
tuple(
  "skiff-service-db-encrypted-field-aad-v1",
  keyId,
  storageServiceId,
  finalPhysicalCollectionName,
  topLevelFieldName,
  logicalStringRecordId
)
```

`tuple` 是磁盘格式的规范组成部分：

```text
tuple(parts) = concat(part(parts[0]), part(parts[1]), ...)
part(text)   = u32be(byte_length(UTF8(text))) || UTF8(text)
```

- 所有字符串按严格 UTF-8 编码；不做 Unicode normalization。
- length 是 4-byte unsigned big-endian，表示 UTF-8 byte length；任何单项超过 `u32::MAX` 都拒绝。
- tuple 不写元素数量；每个调用点的元素数量和顺序由上面的 V1 常量固定。
- record id 来自已经通过 type validation 的逻辑 `string` primary key。insert body、key selector、replacement body 和 Mongo `_id` 都必须映射为相同 UTF-8 字节；runtime metadata 再验证 `_id` 是 BSON string。
- `keyId` 同时进入 HKDF info 和 AAD。即使运维错误地给两个 id 配置相同 root bytes，修改 envelope keyId 也不会保留认证有效性。

不使用 standalone BSON value serialization，因此 BSON library 升级不会改变 KDF/AAD bytes。

AAD 不存进 envelope。collection/field rename、package collection mapping 改变或 storage service id 改变会使旧密文无法解密；它们属于显式存储迁移，不能静默兼容。

### 5.3 Mongo envelope

encrypted field 的物理 BSON 值是 Skiff 保留子文档：

```javascript
{
  _skiff_encrypted: {
    version: 1,
    keyId: "2026-01",
    nonce: BinData(0, "..."),
    ciphertext: BinData(0, "...")
  }
}
```

规则：

- `_skiff_encrypted` 使用现有 `_skiff_` reserved namespace，业务 BSON 无法伪造。
- `version` 决定算法和 envelope 解析规则；V1 只接受整数 `1`。
- `keyId` 是非 secret 操作元数据，最大 64 bytes，只允许 `[A-Za-z0-9._-]`。
- nonce 必须恰好 12 bytes。
- `ciphertext` 使用 RustCrypto/AEAD 通用布局：`encrypted_plaintext || tag`；AES-GCM tag 固定为最后 16 bytes。对空 plaintext，ciphertext 仍恰好 16 bytes；本 V1 string 非空与否均使用同一布局。
- 缺字段、多字段、错误 BSON type、未知 version、未知 key id、认证失败或 UTF-8 失败全部返回同一类 sanitized DB decode error。
- 不接受明文 string fallback，也不把 malformed envelope 当普通 record 解码。

### 5.4 Normative golden vector

实现必须把以下 vector 固化为跨模块 golden test，不能在测试运行时用被测 encode 函数生成 expected value：

```text
rootKey =
  000102030405060708090a0b0c0d0e0f
  101112131415161718191a1b1c1d1e1f
keyId = "test-key"
storageServiceId = "example.com/credential"
collectionName = "provider_credential"
fieldName = "apiKey"
recordId = "credential-1"
nonce = a0a1a2a3a4a5a6a7a8a9aaab
plaintext UTF-8 = "sk-test-secret"

HKDF info =
  00000028736b6966662d736572766963652d64622d656e637279707465642d
  6669656c642d686b64662d763100000008746573742d6b6579000000166578
  616d706c652e636f6d2f63726564656e7469616c0000001370726f76696465
  725f63726564656e7469616c000000066170694b6579

derived AES key =
  15722659746450f313d9413d806470a0511cac22c50bc79b443ca7f6bdbf2e82

AAD =
  00000027736b6966662d736572766963652d64622d656e637279707465642d
  6669656c642d6161642d763100000008746573742d6b657900000016657861
  6d706c652e636f6d2f63726564656e7469616c0000001370726f7669646572
  5f63726564656e7469616c000000066170694b65790000000c63726564656e
  7469616c2d31

ciphertext || tag =
  1548f0565590927edc1d7255016bc39fa33ec16e1c0a917a2cb69ceb15f8
ciphertext = 1548f0565590927edc1d7255016b
tag = c39fa33ec16e1c0a917a2cb69ceb15f8
```

vector 使用 HKDF-SHA256 的 extract+expand、§5.1 的 salt/info、§5.2 的 AAD，以及 AES-256-GCM。Rust unit test 必须验证 tuple bytes、derived key、AAD、combined ciphertext 和 decrypt；另用独立已知实现生成的 fixture 文件保存 expected hex。

### 5.5 Threat boundary

本功能防护：

- Mongo/备份/磁盘副本泄漏；
- ciphertext bit flip；
- encrypted field 在不同 record/service/collection/field 间复制；
- 数据库写权限攻击者把其他 record/field/context 的合法 ciphertext 替换到当前字段。

本功能不防护：

- 已控制 runtime 进程或能读取 keyring 文件的攻击者；
- 业务代码主动记录/返回解密后的 string；
- 上游收到 API Key 后的服务商侧泄漏；
- 进程 memory dump。
- 数据库写权限攻击者把同一 record/field/context 的历史合法 envelope 回放。V1 没有数据库之外的可信 freshness state，回放会认证成功。

## 6. Runtime keyring

### 6.1 Runtime config

keyring 属于 runtime host，不属于 router 或 service activation。`runtime.yml` 增加：

```yaml
serviceDb:
  encryption:
    keyringFile: /run/secrets/skiff-service-db-keyring.json
```

相对路径按 `runtime.yml` 所在目录解析。router `serviceDb` 仍只有 `mongoUrl`；control frame、resolved service config 和 artifact 中不增加 key material 或 keyring path。

### 6.2 Keyring file

```json
{
  "format": "skiff-service-db-keyring-v1",
  "activeKeyId": "2026-01",
  "keys": {
    "2026-01": "<base64-32-bytes>",
    "2025-02": "<base64-32-bytes>"
  }
}
```

加载规则：

- `format` 必须精确匹配；
- `activeKeyId` 必须存在于 `keys`；
- key id 满足 §5.3 约束且不能重复；keyring loader 使用能报告 duplicate object key 的 visitor/parser，不能先反序列化成会覆盖重复项的普通 map；
- JSON object 拒绝未知字段；每个 key entry 的 value 必须是 string；
- key material 使用 RFC 4648 standard Base64 alphabet（`A-Z a-z 0-9 + /`），必须包含 canonical padding。32 bytes 必须编码为恰好 44 chars 并以单个 `=` 结尾；拒绝 URL-safe、无 padding、空白和 decode 后再 encode 不相等的表示；
- 配置了 keyring path 但文件为空、缺失、权限无法读取或 shape 非法时，runtime 启动失败；
- Unix 上文件必须是 regular file，且 group/other permission bits 为 `0`（推荐 `0400` 或 `0600`）；不满足时拒绝启动；
- root/derived key 使用 zeroizing secret container；keyring/cipher 不实现会打印 material 的 `Debug`/`Display`，parse/decode error 不包含原字符串；
- keyring 启动时加载一次，修改后通过 runtime restart 生效。

如果 runtime 未配置 keyring：

- 不含 encrypted field 的 service 正常激活；
- 含 encrypted field 的 service DB provider build fail closed，service 不注册可用路由；
- 不生成临时 key，不把字段降级为明文。

### 6.3 Runtime/provider wiring

`RuntimeFileConfig` 解析 keyring path，runtime driver 构造带 `Arc<DbEncryptionKeyring>` 的 `MongoServiceDbProviderFactory`。factory 在 `DbProviderBuildInput` 已有的 `storage_service_id` 上下文中创建 `ServiceDbRuntime`，router 仍只下发 `mongoUrl + storageServiceId`。

`ServiceDbConfig` 增加可选 keyring/cipher handle；`ServiceDbMetadata::from_runtime_program_db` 在发现 encrypted field 时要求 handle 存在。cipher handle 只保存在 runtime/service-db memory，不进入 request heap。

同一 storage service 的所有 runtime replica 必须挂载同一 keyring。一个 runtime keyring 同时服务该 runtime host 激活的多个 `storageServiceId`；它不是 per-service keyring。keyring 丢失等同于密文不可恢复；数据库备份与 keyring 备份必须分开管理。

本文把所有加载同一 keyring material、以及曾用其中 root key 写入密文的 runtime process、writer 和 `storageServiceId` 集合称为 **keyring deployment cohort**。active key 切换和旧 key 删除的作用域是整个 cohort，而不是单个 service。部署系统必须维护 cohort inventory；无法证明 inventory 完整时，只能添加新 key用于未来写入，不能从在线 keyring 删除旧 key。

runtime 为完整 keyring 计算非 secret commitment：

```text
keyringFingerprint = lowercaseHex(SHA256(
  binaryTuple(
    "skiff-service-db-keyring-fingerprint-v1",
    format,
    activeKeyId,
    sortedByKeyId(keyId, raw32ByteRootKey)...
  )
))
```

`binaryTuple` 与 §5.2 相同地使用 `u32be(length) || bytes`，但 entry 的 root key part 是原始 32 bytes 而不是 UTF-8；entries 按 key id 的 UTF-8 byte lexicographic order 排序。fingerprint 覆盖 active id、全部 old/new key id 和对应 material。root key 是均匀 256-bit 随机值，公开 SHA-256 commitment 不提供可行的离线恢复；fingerprint 仅用于 replica 一致性核对，不参与 KDF/AAD。

规范展开为：前三个 part 依次是 marker、format、activeKeyId 的 UTF-8；之后对每个按 key id 排序的 entry 追加两个 part：`UTF8(keyId)`、`rawRootKey32Bytes`。每个 part 都带独立 4-byte big-endian length，hash 输入中不含 JSON whitespace、object order 或 Base64 文本。

fingerprint golden vector 复用 §5.4 的 `rootKey`，keyring 只有 `test-key` 且它是 active key：

```text
fingerprint input =
  00000027736b6966662d736572766963652d64622d6b657972696e672d6669
  6e6765727072696e742d76310000001b736b6966662d736572766963652d64
  622d6b657972696e672d763100000008746573742d6b657900000008746573
  742d6b657900000020000102030405060708090a0b0c0d0e0f101112131415
  161718191a1b1c1d1e1f
keyringFingerprint =
  5564d5ae1d4db8977a0eb16bf7b88a27d31bd50e390043388bb89fe05561df3c
```

该值进入 keyring unit golden test，expected hex 不能由被测 fingerprint 函数动态生成。

runtime 成功加载 keyring 时发一条不含路径和 material 的结构化启动事件：

```text
event = "service_db.encryption_keyring_loaded"
format = "skiff-service-db-keyring-v1"
activeKeyId = "..."
keyringFingerprint = "<64 lowercase hex>"
```

该事件是 V1 多 replica rotation 人工屏障的核对依据；相同 active id 但不同 root material、缺少旧 key 或额外 key 都会产生不同 fingerprint。不把这些字段加入 router control protocol 或 service API。

### 6.4 Local instance 与 test runner

- `skiff instance init/up` 在 instance `dev-home/secrets/` 中 ensure 一个持久 keyring，使用 lock + same-directory temporary file + fsync + atomic no-clobber install，首次生成后不覆盖，文件 mode 设为 `0600`，runtime config 引用该路径；并发 `init/up` 的 loser 必须读取 winner 的完整文件，不能覆盖或读到半写内容。
- 同一 instance restart 保持 key；删除整个 instance dev-home 明确意味着丢弃本地 encrypted data 的解密能力。
- test-runner 为每个临时 runtime root 生成临时 keyring；package-test DB 的 storage service id 本来就是 run-scoped，因此不同 test run 不共享密文。
- production deploy 脚本不复制 keyring。运维必须先在远端 secret mount 上 provision 文件，再启动 runtime。
- `runtime/runtime.example.yml` 和 canonical DB reference 记录配置与灾难恢复规则，但示例不包含真实 key。

## 7. Service DB mapping 实现

### 7.1 统一 field storage pipeline

在现有类型/BSON projection 外增加一层 storage mapping：

```text
write:
RuntimeValue / business JSON
  -> existing type projection
  -> plaintext BSON string
  -> encode_field_storage(field, encrypted_context?, bson)
  -> encrypted BSON envelope

read:
encrypted BSON envelope
  -> decode_field_storage(field, encrypted_context?, bson)
  -> plaintext BSON string
  -> existing type/result projection
  -> RuntimeValue / business JSON
```

新增共享 helper（命名可按模块调整）：

```rust
struct EncryptedRecordContext<'a> {
    record_id: &'a str,
}

encode_field_storage(
    field: &DbFieldMetadata,
    encrypted_context: Option<&EncryptedRecordContext<'_>>,
    plaintext: Bson,
) -> Result<Bson>

decode_field_storage(
    field: &DbFieldMetadata,
    encrypted_context: Option<&EncryptedRecordContext<'_>>,
    stored: Bson,
) -> Result<Bson>
```

`Identity` 原样返回且完全忽略 `encrypted_context`；`Encrypted` 要求 `Some(context)`，只接受/产生 `Bson::String` 与 envelope。JSON 路径和 RuntimeValue 路径都必须在 existing type projection 之后调用这两个 helper，不能各自调用 crypto。

string primary-key 限制只适用于 `DbCollectionMetadata::has_encrypted_fields() == true` 的 object。未声明 encrypted storage 的普通 DB object 继续使用现有任意合法 key 类型和 BSON mapping，不能因为共享 helper 引入行为变化。

### 7.2 读路径重构

当前 read helper 会先移除 `_id` 再逐字段 decode。对于含 encrypted field 的 collection，本阶段先要求实际 stored `_id` 是 BSON string，构造 `EncryptedRecordContext` 并传给 field storage decode；普通 collection 继续走原有 key decode：

- `business_value_from_document`；
- `runtime_business_value_from_document`；
- projected find one/many；
- update/replace/upsert 返回文档。

只有最终 materialized business object 才把 `_id` 映射回 key field。encrypted envelope 必须在递归 business BSON validation 前被识别并解密，否则 reserved `_skiff_` 子文档会被误判为业务对象。

Mongo projection 沿用现有 contract，始终注入 `_id` 并在逻辑 projection 中返回 primary key。尤其 `fields { apiKey }` 的物理查询必须读取 `_id`，结果必须 materialize `{ id, apiKey }`；business JSON 与 RuntimeValue 两条 projection test 都锁定这一形状。

### 7.3 完整写路径

对于含 encrypted field 的 collection，document insert/replace 先验证逻辑 key 是 string，并把同一 `record_id` 同时编码为 BSON `_id`、放入 `EncryptedRecordContext`；普通 collection 不增加这项验证：

- `document_from_business_value`；
- `document_from_runtime_business_value`；
- `documents_from_business_values`；
- replacement business/runtime variants；
- upsert `$setOnInsert`。

同一次 insert many 中每条 record 独立生成 nonce。replace 返回/文件 cascade 逻辑看到的是 envelope 文档，但 encrypted V1 不允许 immutable file，因此不加入 cascade paths。

### 7.4 Change 写路径

`change_update_document` 与 runtime variant 增加 `encrypted_context: Option<&EncryptedRecordContext>`：

- key selector：先转换 selector key，再构造 `$set`，传 `Some(context)`；
- upsert by key：传 selector key；
- query selector/update many：传 `None`；触碰 encrypted set 时返回 operation-not-supported；
- identity 字段维持现状；
- encrypted field 只允许完整 top-level `Set`。

这要求调整当前“先编码 update，再解析 selector”的顺序。对于 key selector，统一 helper 先把 selector 校验/规范化为逻辑 string `record_id`，再同时生成 Mongo `_id` filter 和 encryption AAD；禁止各路径自行从 BSON/JSON 反解。replace by key 继续要求 body 不携带 key；replace by query 的完整 body key 是 AAD 权威值，Mongo 若选中不同 `_id` 则 replacement 失败，不能产生可读但绑定错误的密文。

### 7.5 Storage use policy

把现有 `validate_recoverable_field_use` 收敛为 `validate_storage_field_use`：

- recoverable envelope 维持现有 top-level projection/set 规则；
- encrypted 使用 §3.3 规则；
- identity 允许现有行为。

`field_path_to_mongo_name`、query filter、order、projection、index reconciliation 和 change builder 都通过这一入口。错误只包含 canonical type/field/use case，不包含 value、ciphertext、nonce 或 key。

## 8. Key rotation 与数据演进

### 8.1 Key rotation contract

V1 只支持停写维护窗口内的 rotation，不承诺在线无停机轮换。安全 runbook：

1. 从部署 inventory 枚举整个 keyring deployment cohort：所有加载该 keyring 的 runtime/writer、它们当前或历史服务的全部 `storageServiceId`，以及每个 storage service 下声明 encrypted 的最终 physical collection/field。inventory 不完整则停止，旧 key 继续在线保留。
2. 阻断 cohort 内所有 service 的新业务写入并 drain 正在执行的请求；迁移期间保持全 cohort 写屏障。
3. 停止 cohort 内所有 runtime replica/writer。
4. 在 cohort 的所有 runtime 上安装完全相同的 old+new keyring，并把 `activeKeyId` 切到新 key。
5. 启动所有 replica 但继续阻断业务写入；收集每个 runtime 进程的 `service_db.encryption_keyring_loaded` 事件，确认 `format + activeKeyId + keyringFingerprint` 都一致，任何旧 replica/writer 存活或缺少事件都中止轮换。
6. 对 inventory 中每个 `storageServiceId + collection` 运行一次服务迁移任务；同一 collection 不按 encrypted field 拆任务。任务只通过普通 service-db API 工作：按 identity string primary key 做 `where id > lastId order id asc limit batchSize` 分页，读取每一行所有 encrypted field 的当前 plaintext，再在同一次按-key update 中把这些字段分别 top-level set 为同一当前值。任务重写全部行，不需要也不能观察 envelope keyId。checkpoint key 固定为 `(targetKeyringFingerprint, storageServiceId, finalPhysicalCollectionName)`；每批 transaction 完成后持久记录 `lastId`。如果写批次后、checkpoint 前崩溃，重复该批只会再次使用 active key 和新 nonce 加密同一当前值，是安全幂等操作。连续两次 rotation 因 target fingerprint 不同不会复用游标。由于持续写屏障，不会覆盖并发业务值。
7. 具备受控 Mongo read-only 运维权限的操作者对 cohort inventory 中每个 encrypted collection/field 执行物理确认。对于 active id `NEW_ID`，以下计数必须为零：

   ```javascript
   db.getCollection("<collection>").countDocuments({
     "<field>._skiff_encrypted.keyId": { $ne: "NEW_ID" }
   })
   ```

   `$ne` 同时匹配缺字段，因此该检查覆盖旧 key、明文、缺失/malformed envelope 的大部分错误形态；migration 的业务全量 read 还会让 malformed envelope fail closed。所有 cohort 扫描计数均为零前不能继续；扫描期间保持全 cohort 写屏障，命令输出和 inventory 一起作为 rotation 记录保存。
8. 再次停止 cohort 全部 replica/writer，从在线 keyring 删除旧 key，启动并用新的共同 fingerprint 确认一致后解除全 cohort 写屏障。旧 key material 放入与数据库备份分离的离线 recovery keyring，直到所有可能包含旧 envelope 的备份过期或完成恢复演练后的重加密，不能立即销毁。

读取旧 key 时不做隐式写回，避免普通 read 产生副作用。V1 不提供扫描/rotation CLI：各服务负责步骤 6 的普通数据迁移，数据库操作者负责步骤 7 的只读物理确认。受控 raw Mongo read access 和完整 cohort inventory 是 V1 rotation 的部署前提，不授予业务 service。不能在有并发写的情况下用普通 read + set 轮换，也不能只迁移一个 `storageServiceId`、只滚动重启部分 replica，或在 cohort 外 writer 未纳管时删除旧 key。

### 8.2 启用 encrypted 的数据前提

对已有非空明文字段直接增加 `storage ... using encrypted` 后，旧 BSON string 会在读取时 fail closed。在已有非空 collection 上新增非 nullable encrypted field，同样会因旧记录缺字段而失败。

V1 只允许：

- 全新的 DB object/物理 collection；或
- 已确认完全为空的 collection。

已有非空 collection 的原地字段切换和“新增 encrypted field”都不受支持。需要保留数据时，服务必须另建使用最终 `storageServiceId + collectionName + fieldName` 的新 DB object/collection，通过正常 encrypted insert 做 out-of-place copy，在停写窗口完成校验与切流；这属于服务 schema migration，不在本能力实现范围。

不提供“看到 string 就现场加密”的 dual read，因为它会把数据库篡改或错误部署伪装成合法迁移，并让运行中的多个 artifact 产生混合写入。缺字段、明文 string 和 malformed envelope 都返回同一 fail-closed decode error。

### 8.3 Rollback

一旦有 encrypted envelope 写入，回滚到不认识 storage metadata 的旧 runtime/artifact 会读到 reserved document 而失败。发布顺序必须是：

1. 先部署支持 encrypted metadata/codec 但尚无 encrypted schema 的 runtime；
2. 确认所有 runtime replica 使用同一 keyring；
3. 再加载含 encrypted field 的 service artifact；
4. 最后由业务写入数据。
回滚 service artifact 前必须停止写入并迁移/清空 encrypted 数据；不能简单删除 storage declaration。runtime binary 可以保留向后能力，不需要回滚。

## 9. 营地原则检查

本功能直接经过的区域有两处现存隐式/重复规则，必须在本次收敛：

1. **JSON 与 RuntimeValue 两套 BSON mapping。** 加密、AAD、envelope parsing 只能放在共享 `encode/decode_field_storage`；两条入口只负责各自现有 type projection。
2. **recoverable 专用 opaque-field policy。** query/order/index/change 已经依赖 `validate_recoverable_field_use`。新增第二套 `validate_encrypted_field_use` 会复制所有调用点，因此本次改成 storage-aware policy 后同时表达 recoverable/encrypted。

以下不在本次清理：

- Date 仍由 `DbBoundaryValuePlan::Date` 做 type-driven BSON projection。把 Date、recoverable、encrypted 全部改造成公开通用 codec 会扩大语言 surface，与“先只实现加密”冲突。
- artifact-model 与 linked-program 保留各自 DTO；它们属于编译/加载 trust boundary，不合并 crate。只确保 storage 字段显式转换和 round-trip 测试。
- 不建立 schema migration framework；当前仓库另有独立演进工作，本功能只定义 encrypted 字段的 fail-closed 前提。

## 10. 实施 DAG、worktree 与提交

实现是跨 syntax/compiler/artifact/runtime/scripts 的高风险改动，使用仓库同级 worktree：

```text
A. Source + artifact contract
   syntax/parser/AST
   source semantic/publication metadata
   lowering/artifact/projection/linker

B. Crypto + runtime keyring
   config/provider wiring
   local/test provisioning

A ─┐
   ├─> C. service-db mapping/use policy/record-key AAD
B ─┘
         └─> D. integration/live tests + canonical docs
```

建议 worktree：

- `/Users/geek/workspace/skiff-db-encrypted-contract`：A；
- `/Users/geek/workspace/skiff-db-encrypted-runtime`：B；
- `/Users/geek/workspace/skiff-db-encrypted-mapping`：C；
- D 在 A/B/C 合并 main 后从最新 main 建 integration worktree，避免验收旧组合。

A 与 B 可以并行；C 依赖 A 的 IR 形状和 B 的 cipher API。每个子任务使用同一份本文档，完成聚焦测试、独立只读验收后提交。合并顺序 A → B → C → D；遇到冲突由后合并任务基于 main 重放并重跑相关测试。全部通过后删除 worktree 和已合并临时分支；未经明确要求不 push。

文档本身是小范围单文件改动，直接在当前 main 提交；实现阶段才使用上述 worktree。

## 11. 测试与验证

### 11.1 Syntax/source/compiler

- parser 接受 canonical syntax 和可选分号。
- parser 拒绝 nested path、未知 codec、缺失 token。
- semantic 拒绝 unknown/duplicate/key/non-string/nullable 字段，以及声明 encrypted field 但 primary key 非 string 的 DB object。
- semantic/runtime metadata 拒绝 encrypted 与 recoverable/immutable-file plan 叠加。
- index fields 和 partial-index where 不能引用 encrypted 字段。
- predicate、regex、order、update-query/update-many set、partial change 产生明确 compile error。
- top-level projection、insert、replace、upsert、key update 编译成功。
- alias-to-string 接受，alias-to-other 拒绝。
- 未声明 encrypted field 的 DB object 继续允许现有非 string primary key，并通过完整读写回归测试。
- service-owned 和 package DB metadata 都保留 `Encrypted`。
- artifact JSON round-trip、artifact identity、linked program conversion 覆盖非 identity storage。

聚焦命令：

```bash
cargo test -p skiff-syntax
cargo test -p skiff-compiler-source
cargo test -p skiff-compiler-lowering
cargo test -p skiff-artifact-model
cargo test -p skiff-runtime-linker
cargo test -p skiff-runtime-linked-program
```

### 11.2 Crypto/keyring unit tests

- valid keyring parse，invalid format/id/base64/length/active id、duplicate JSON key 和不安全 Unix permission 拒绝。
- §5.4 normative vector 的 tuple、HKDF、AAD、combined ciphertext/tag 和 decrypt 字节完全一致。
- 同一 plaintext/AAD 连续写两次产生不同 nonce/ciphertext，均可解密。
- ciphertext、nonce、tag、AAD、record key、service id、collection、field、root key 任一变化均失败。
- malformed/unknown version/unknown key id 返回 sanitized error。
- old key envelope 可读，新写只用 active key。
- keyring fingerprint 对 JSON key order 稳定；相同 id 但不同 root、缺少旧 key、active id 不同都会变化，完全相同 keyring 在不同 replica 上相等。
- §6.3 fingerprint golden vector 字节完全一致。
- unique sentinel plaintext/key material 不出现在 envelope、`Debug`、error string 或 tracing capture。

### 11.3 Service-db mapping tests

两条 mapping 路径都覆盖：

- business JSON insert/read；
- RuntimeValue insert/read；
- insert many 每行不同 nonce；
- replace by key/query；
- upsert insert/change；
- update by key set；
- update by query/update many set 拒绝；
- full read 与 `fields { apiKey }` projection 解密；后者的物理 projection 和逻辑 `{ id, apiKey }` 结果都包含 primary key；
- predicate/order/index/runtime-forged metadata 拒绝；
- 从 Mongo raw `Document` 断言 sentinel 明文不存在；
- 把 A record envelope 复制到 B record 后解密失败；
- package DB end-to-end 使用 projection 后的最终 physical collection 和 service `storageServiceId` 生成 AAD，默认/映射 collection 两条路径都覆盖；
- identity、Date、recoverable 和 immutable-file 现有测试不回归。

聚焦命令：

```bash
cargo test -p skiff-runtime-service-db --no-fail-fast
cargo test -p skiff-runtime-host --no-fail-fast
```

### 11.4 Config/test-runner/live tests

- runtime config 相对/绝对 keyring path 解析。
- 无 keyring + 普通 service 正常；无 keyring + encrypted metadata 激活失败。
- router control frame 不含 keyring path/key material。
- local instance ensure keyring 且 restart 不改 key。
- 两个并发 ensure 不覆盖 key、不产生半写文件，最终都读取同一完整 active key。
- test-runner 临时 runtime 可执行 encrypted service test。
- Mongo live test 写入 sentinel，业务 read 相等，raw Mongo document 不含 sentinel。
- runtime 重启后使用同 keyring 仍可读；换 root key 或删旧 key 后 fail closed。
- rotation fixture 至少包含两个 `storageServiceId` 共用同一 keyring，且至少一个 collection 有两个 encrypted field：在 cohort 写屏障下，每个 collection 单次扫描并按行同时重写全部 encrypted field，再对所有 collection/field 执行 §8.1 的 raw Mongo `$ne` count 归零检查；遗漏任一 storage service/field 时禁止删除旧 key。测试覆盖批次写入后、checkpoint 前崩溃恢复，以及下一次 rotation 使用新 target fingerprint 不复用旧游标。完整迁移后删除在线旧 key 仍可读，离线 recovery keyring 保留旧备份恢复能力；文档明确不声称并发在线 rotation。

live 验证使用隔离 harness；它在 `45000`–`45999` 租用端口，启动并清理独立 Mongo、router、runtime、
keyring 和 artifact root，不接触 stable instance：

```bash
node scripts/check-db-encrypted-storage-live.mjs
```

最终验证：

```bash
pnpm test
cargo test --workspace --no-fail-fast
```

### 11.5 Downstream smoke

本阶段不改 Agine，但 encrypted DB 能力会成为 BROK 前置项。合并到主工作区 stable runtime 后，先运行一个独立 fixture service 验证物理密文；等 BROK 接入 ProviderCredential 时，再按 workspace 约定补跑 Agine `npm run e2e:chat-smoke`。不能用尚未实现的 BROK 作为本阶段唯一验收。

## 12. 发布与验收标准

全部满足才算 encrypted storage field 完成：

1. `type` 与普通表达式类型系统完全不出现 encryption/secret wrapper。
2. canonical `storage apiKey using encrypted` 从 parser 到 runtime metadata 不丢失。
3. V1 只接受顶层非 nullable string 且所属 DB object 使用 string primary key；该限制不影响没有 encrypted field 的现有 DB object；所有不支持的 query/index/update/codec 组合在 compiler/runtime fail closed。
4. service-db 两条 mapping 路径共享同一 storage encode/decode/use policy，没有复制 crypto 或 envelope 逻辑。
5. Mongo raw document、数据库备份等价表示和错误/log capture 中没有 plaintext sentinel。
6. HKDF/AAD/ciphertext 按 §5 字节协议和 golden vector 固定；AES-GCM nonce 随机，AAD 绑定 key id/storage service/collection/field/string record id，篡改与跨上下文复制测试失败；同上下文历史回放明确不在防护范围。
7. keyring 不经过 router/service/artifact，缺 keyring 时 encrypted service 不激活，普通 service 不受影响；全量 keyring fingerprint 能发现 replica 同 id 不同 material 或缺 key。
8. old key 可读、active key 写入、runtime restart 可读；rotation 覆盖共享 keyring 的完整 deployment cohort，并在全 writer 停写屏障下由各业务全量重写 + 运维 raw Mongo 归零检查完成；inventory 不完整时旧 key 不得删除，不提供危险的 plaintext fallback 或在线轮换承诺。
9. local instance、test-runner、runtime example 和 `doc/reference/db.md` 同步完成。
10. 聚焦测试、`pnpm test`、workspace Rust tests、Mongo live test 和独立验收通过。
11. 实现分支分别提交并合并本地 main，worktree/临时分支清理完成；不擅自 push。
