# Record Spread 与 Contract Storage 实现文档

日期：2026-08-05
状态：plan（Phase 0 已完成）

## 引用链

- 权威语法与语义：`doc/reference/record-spread.md`（`spread` 字段复制）。
- 权威存储契约：`doc/reference/db.md §1.3`（`db contract` + `db object ... implements`）。
- 跨包类型引用双拼写：`doc/reference/static-semantics.md §10`。
- 现有实现参照：`doc/implementation/db-encrypted-storage-field.md`（db object storage mapping 的
  source/artifact/runtime 契约先例）。

## 0. 结论

为"引擎声明类型、宿主声明存储"的共享集合模式实现两个语言特性：

1. `spread`：record 声明中复制另一个 record 的字段（编译期展开、无类型关系、快照语义）。
2. 存储契约：引擎侧 `db contract`（类型 + 主键 + 必需索引，不 claim 物理集合）+ 宿主侧
   `db object ... implements <contract>`（完整字段、索引、storage mapping，物理身份归宿主），
   链接/激活期做字段/主键/索引/存储语义覆盖校验。

先决条件"跨包类型引用接受斜杠拼写"（`alias/module.Symbol`）已实现并合入（commit a32b5e59，
Phase 0）。

## 1. 目标与非目标

### 1.1 目标

1. 产品类型（宿主）可用 `spread` 复用引擎类型字段，两个类型之间无子类型/赋值/传参关系。
2. 引擎与宿主共享同一物理集合：引擎按契约字段子集读写，宿主按完整字段集读写，无共享可变字段镜像。
3. 覆盖校验在链接/激活期 fail closed：字段覆盖（schema identity 逐字段一致）、primary key 一致、
   必需索引覆盖、storage mapping 与 recoverable 语义一致、契约恰好一个实现。
4. 共享集合写入限制：宿主执行 insert，引擎只读 + field-scoped update，禁全文档 replace/upsert。

### 1.2 非目标

- 不做方法/interface conformance 复制（`record-spread.md §5`）。
- 不做通用类型兼容性规则变更（不引入子类型系统）。
- 不做跨 service 数据复制或共享。
- 不做 `db contract` 之外的其它存储绑定形态（如泛型 db object）。

## 2. 已完成的 Phase 0：跨包类型引用斜杠拼写

commit `a32b5e59`。改动：

- `compiler/source/src/type_resolution_model/query.rs`：`resolve_package_type_symbol_path` 的
  Public 视图从 `!path.contains('/')` 放宽为接受点/斜杠两种拼写（斜杠由 `PackageExportResolver`
  解析期归一化为 public path；视图由 alias 名决定，TopLevel 视图不变）。
- 错误信息更新（不再声称斜杠不可用）。
- 回归测试：`public_and_top_level_views_are_isolated_and_emit_one_canonical_dependency_ref`
  增加 `provider/Bindings` ≡ `provider.Bindings` 断言。
- `doc/reference/static-semantics.md §10` 补充双拼写规则。

## 3. Phase 1：spread 语法与语义层展开

### 3.1 语法（syntax crate）

- `syntax/src/parser/decl.rs` `parse_type_decl`：record 声明新增独立字段
  `TypeDecl.spreads: Vec<TypeRef>`（**不放 `FieldDecl` 变体**——避免连累所有 `FieldDecl { name, ty }`
  解构点如 db_attachment / lowering；`spread` 条目在 `parse_field_block` 内以
  `check_db_field_entry` 同款 ident+`:` lookahead 消歧）。
- contextual keyword 消歧：`spread` 后跟 `:` 时按普通字段名解析（与 db block 内 `where` 先例一致）。
- `spread` 只允许出现在 record 形态字段列表内；representation / union / alias / interface 声明体
  中出现即 parse error（`parse_field_block` 只在 record 分支调用，自动成立）。

### 3.2 语义层展开（compiler/source）

- **展开 pass 插入点与产物**：在 `parsed_sources::build_parsed_sources` 之后、类型解析之前新增
  展开 pass；产物是**展开后的 AST**（`TypeDecl.fields` 直接含复制字段，`spreads` 清空）。所有消费
  字段集的机制由此自然拿到展开后字段集：`package_db_schema/mod.rs:57 validate_package_db_schema`
  （`build_from_linked` 首步）、`semantic/db_attachment.rs`（附着校验、primary key、storage
  mapping）、`type_resolution_model`（`source_type_resolution` model.rs:2360、`resolve_constructor_target`
  query.rs:593）、`package_rules/type_validation.rs:72`（std 名违例）、`compile_model.rs:578`
  （interface-value violations）、actor 附着字段（model.rs:1766）、`alias_resolution.rs:219` 与
  `root_refs` 的 AST 规约 pass。
- **字段类型文本的限定规则（关键）**：`source_type_resolution` 把字段存为类型文本，按**源模块**
  上下文解析（query.rs:899-923）。复制时必须同时完成限定，否则照抄进目标 record 会在目标模块
  上下文解析错位：
  - 同包跨 module 的裸类型名转 `root.<源模块>.<Name>`；
  - 跨包引用保持 alias 限定（点或斜杠拼写，Phase 0 已支持）；
  - 泛型源先按实参替换源 type_params 再限定（复用
    `compiler-core::type_ref::substitute_type_params_in_type_ref_ref`，`db_lowering.rs` 已有先例）。
- 展开规则（对齐 `record-spread.md §3`）：
  - 源解析后必须是 record 形态（名义 record；透明 alias 展开为 record 的合法）；representation /
    union / interface 报错。
  - 泛型源：只允许 fully instantiated（实参闭合、不引用目标类型参数）。
  - 字段重名（spread 之间、spread 与显式字段）compile error；自 spread / 循环链按 source 引用图
    检测报错。
  - 不复制 impl 方法、interface conformance、type namespace。
- 展开结果按源类型缓存复用，避免同一源被多次 spread 时重复解析。

### 3.3 验收

- parse / semantic 单元测试覆盖：基本复制、多 spread、重名、自/循环 spread、泛型源（闭合与未闭合）、
  非 record 源、`spread:` 字段名消歧、同包与跨包源、alias 链源。
- 已有测试不回归（`cargo test -p skiff-compiler-source --lib` 全量绿）。

## 4. Phase 2：spread lowering 与 db 附着集成

### 4.1 lowering（compiler/lowering）

- `compiler/lowering/src/declaration_lowering.rs`：`lower_type_decl_descriptor`（L214-272）消费
  展开后的 AST 字段集生成 `TypeDescriptorIr::Record`；**IR 中不存在 spread 节点**（`TypeDeclIr` /
  `TypeDescriptorIr` 不变）。
- lowering 的数据来源是 §3.2 展开 pass 的产物（展开后 AST），不自己展开。

### 4.2 db object 附着

- 含 spread 的 record 满足附着约束（非泛型 concrete record）后可正常附着 db object；spread 复制
  字段与显式字段同等参与 primary key / 索引 / storage mapping 声明。

### 4.3 验收

- 含 spread 的 record 附着 db object 的端到端 fixture 测试（skiff-tests 或 implementation-tests）。

## 5. Phase 3：db contract 声明（引擎侧）

### 5.1 语法与解析

- `syntax/src/parser/db.rs` `parse_db_decl`：新增 `db contract Name { ... }` 形态，与 `db object`
  共享 primary key / index entry 语法。
- 附着规则：`db contract Name` 必须附着到同模块同名 `type`（复用现有附着校验，`db_attachment.rs`）；
  附着类型必须是非泛型 concrete record。
- 契约不产生物理 collection identity，不参与 `(packageId, logical identity)` 编码。

### 5.2 artifact / lowering

- `compiler/lowering/src/db_lowering.rs` `lower_db_declarations`（L292-466）：契约声明新增
  `DbObjectKindIr::Contract` 变体；`DbDeclarationIr.collection_name` 改为**可选**（契约无
  collection，`db object` 保持非空）。契约携带主键与必需索引 identity。
- **schema 版本**：bump `artifact-model/src/schema.rs` 的 `FILE_IR_SCHEMA_VERSION`（当前 v11）与
  `PACKAGE_ARTIFACT_SCHEMA_VERSION`（当前 v10）常量，并同步精确字符串断言（
  `source_file_lowering/tests.rs`、`projection/tests.rs`）与 loader content validation。
- 契约类型上的 db 语句照常 lowering，db target 引用契约 identity。

### 5.3 验收

- 契约声明的 parse / lowering 单元测试；契约类型上的 db 语句可编译；契约不产生物理集合编码。

## 6. Phase 4：db object implements 与覆盖校验

### 6.1 语法与解析

- `db object Name implements <contract-ref> { ... }`：`<contract-ref>` 是跨包类型引用（点或斜杠
  拼写，Phase 0 已支持）；指向非契约附着类型或 interface 时 fail closed。
- `implements` 复用现有关键字，在 db decl 内新增位置；与 interface conformance 的 `implements`
  按出现位置消歧。

### 6.2 覆盖校验（编译 vs 激活分工）

五项校验分两个阶段，全部 fail closed、不进入业务 catch：

**宿主编译期（driver / linker，契约类型 facts 在宿主 package artifact 解析中可用）：**

> driver 收敛：生产服务编译时，`canonical_dependencies::foreign_db_metadata` 只对 `db object
> ... implements` 子句实际引用的依赖包加载 canonical File IR（别名白名单来自宿主源码 AST，
> 拼写规则与 lowering 的 `resolve_implements_contract` 一致；契约 facts 面只含 `db contract`
> 声明）。未被引用的依赖包不加载；test service 的 topLevelAlias 视图保持不变。参考文件：
> `compiler/driver/source_compile/canonical_dependencies.rs`。

1. **字段覆盖**：实现类型字段集 ⊇ 契约类型字段集；重叠字段 schema identity 逐字段一致
   （按 `static-semantics.md §16/§17`；spread 复制的字段 identity 天然一致，宿主手写同形本地
   名义类型不构成一致）。
2. **primary key 一致**：契约与实现 primary key 同字段、同类型。
3. **存储语义一致**：重叠字段的 storage mapping（encrypted）与 recoverable 语义一致。

**激活期（host admission）：**

4. **必需索引覆盖**：契约必需索引翻译到宿主 collection 的 canonical spec 后 ⊆ 实现索引
   （翻译维度：`canonical_managed_index_spec(package_id, logical_collection, name, keys, unique)`
   ——契约索引无物理 collection，翻译到宿主 collection 是新增维度）。
5. **恰好一个实现**：同一契约在同一 service assembly 内恰好被一个实现声明覆盖；缺失/重复 fail
   closed，不因契约 target 是否被实际使用而豁免。
6. 索引 identity 合并复用 `runtime/service-db/src/index.rs` 现有机制：
   `ServiceDbIndexProvisionPlan::from_runtimes`（L119）/ `merge_collection_plan`（L248）/
   `collection_index_plan`（L210），由 `provider.rs:39` 触发。

### 6.3 验收

- 校验的单元测试：字段缺覆盖、类型不一致、key 不一致、索引缺失、storage 语义不一致、多实现、
  无实现、指向非契约类型。
- 与 `db.md §1.2` 激活期索引合并机制的交互测试（多 version 合并、同 identity 不同定义拒绝）。

## 7. Phase 5：runtime 绑定解析与写入限制

### 7.1 绑定表

绑定表在宿主侧 DB 能力构建时生成并随 assembly 生命周期存在：

- **契约声明必须从物理集合编码中排除**：`runtime/host/src/loader/active_assembly_context.rs` 的
  `candidate_db_metadata`（L350）→ `activation_db_metadata`（L760-860）遍历
  `file.declarations.db` 生成 `DbProviderTargetMetadata`（其 `collection_name` 来自
  `DbDeclarationIr.collection_name`）。契约（`DbObjectKindIr::Contract`、collection 为 None）不得
  生成 provider metadata，不参与索引计划；只有 `db object` 实现生成。
- **绑定表**：契约 DbObjectTargetId → 宿主实现 target 的映射在 `active_assembly_context::build`
  期间构建（契约与实现的 package/artifact 解析不一致 fail closed）。
- **两个消费路径**：
  - eval：`runtime/eval/src/assembly_execution/projection.rs` `resolve_db_target`（L85-152 /
    `resolve_db_declaration` L636-674）——契约 db target 经绑定表解析到宿主集合；
  - capability store：`exact_db_target_lookup_key`（`runtime/capability-context/src/db.rs:332`，
    key scheme 前缀 `skiff-db-object-target-v1`）——契约 target 必须解析到同一宿主 key，否则
    store 查找错位。**结论：key scheme 无需新版本**——契约→宿主 remap 发生在 key 计算之前
    （`db_eval.rs` 的 `db_capability_target` 在构造 `DbCapabilityTarget`（key 在此计算）之前完成
    remap），store 里只存在宿主 target 的 key，契约 target 永不直接入 key。
- `db_eval.rs:326` 的 recoverable plans 路径同样走 `resolve_db_target`，自动继承绑定。

### 7.2 解码

- 引擎按契约字段子集 plan 解码，宿主按完整字段集解码；视图未声明的文档字段忽略
  （durable read 现有行为，`runtime/service-db/src/tests/recoverable.rs` 已有测试先例）。

### 7.3 写入限制

- **compiler 侧双重拒绝点**：`expression_type_model/db_typing.rs`（`db_operation_type` /
  `validate_db_change_path`）——契约 target 上的全文档 replace/upsert、以及引擎（契约视图）对
  共享集合的 insert，直接 compile error；lowering 侧 `db_lowering.rs` 二次拒绝。
- **runtime 侧**：绑定表携带"契约视图"标志；`execute_db_command` 对契约视图执行
  replace/upsert/insert 时 fail closed。
- 创建路径由宿主在同一 `db transaction` 内插入初始行并调用引擎初始化操作（事务动态作用域已支持
  引擎调用参与宿主事务）。

### 7.4 验收

- runtime 层集成测试：引擎读宿主写入的行（多余字段忽略）、引擎 field-scoped update 不触碰宿主
  字段、宿主 insert 后引擎初始化、replace/upsert 被拒。

## 8. Phase 6：回归与端到端

- agine 场景回填：以 `spread` 重写产品 Thread 类型 + 宿主 db object 实现契约（可先在
  internals 分支验证），确认 accept/list 路径语义不变。
- 全量 `pnpm verify --only tests`（或聚焦 compiler/runtime selector）无回归。
- 文档一致性：实现过程中如与 `record-spread.md` / `db.md §1.3` 目标语义有出入，先改权威文档再改
  实现。

## 9. 涉及文件映射表

| Phase | 文件 | 改动 |
| --- | --- | --- |
| 0（已合入） | `compiler/source/src/type_resolution_model/query.rs` | Public 视图双拼写 |
| 0（已合入） | `doc/reference/static-semantics.md §10` | 双拼写规则 |
| 1 | `syntax/src/parser/decl.rs` | `TypeDecl.spreads`、`parse_field_block` 分支与消歧 |
| 1 | `compiler/source/src/parsed_sources/` | 展开 pass（产物=展开后 AST） |
| 1 | `compiler/source/src/package_db_schema/mod.rs` | 消费展开后字段集 |
| 1 | `compiler/source/src/type_resolution_model/` | 展开后字段集、跨模块限定 |
| 2 | `compiler/lowering/src/declaration_lowering.rs` | 展开后 AST → Record IR |
| 3 | `syntax/src/parser/db.rs` | `db contract` 解析 |
| 3 | `compiler/lowering/src/db_lowering.rs` | Contract kind、collection 可选化 |
| 3 | `artifact-model/src/schema.rs` | FILE_IR / PACKAGE_ARTIFACT schema 版本 |
| 4 | `runtime/service-db/src/index.rs` | 必需索引翻译与合并 |
| 5 | `runtime/host/src/loader/active_assembly_context.rs` | 契约排除、绑定表构建 |
| 5 | `runtime/eval/src/assembly_execution/projection.rs` | resolve_db_target 绑定消费 |
| 5 | `runtime/capability-context/src/db.rs` | lookup key 绑定消费 |
| 5 | `compiler/source/src/expression_type_model/db_typing.rs` | 写入限制编译期拒绝 |

## 10. 风险与开放问题

1. **语义层展开的改造面**：展开 pass 必须早于 `validate_package_db_schema` 与类型解析（Phase 1
   的第一优先级）；字段类型文本的跨模块限定是最大的正确性陷阱（§3.2）。
2. **契约 artifact 形态**：`DbObjectKindIr` 新变体 + `collection_name` 可选化 + schema 版本
   bump（§5.2）——契约声明若漏了从 provider metadata 排除，会默认生成物理集合编码（§7.1）。
3. **激活期索引合并**：契约必需索引翻译到宿主 canonical spec 是新的合并维度
   （`canonical_managed_index_spec`），Phase 4 验收必须覆盖（§6.2）。
4. **测试 topLevelAlias 与契约**：`kind: test` service 经 topLevelAlias 引用契约类型的场景
   明确不支持（契约 target 无物理集合），test 不承担宿主角色。
5. **spread 与 recoverable**：展开不改变字段 lane 判定；跨 request 边界的 fail-closed 约束照常
   生效（`db.md §11`）。
