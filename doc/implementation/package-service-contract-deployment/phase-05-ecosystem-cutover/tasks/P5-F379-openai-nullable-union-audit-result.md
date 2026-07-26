# P5-F379 OpenAI nullable-union audit result

状态：**TASK_EXECUTABLE**。现有语言规则已经唯一决定修复方向：OpenAI source 的
`OpenAiImageFormat?` 是合法且准确的 source type；`OpenAiImageFormat` 是透明 alias，因此其语义类型是
外层 `Nullable` 包住三分支 literal union。当前 compiler/source 在解析本地 callable signature 时先把
alias 展开成无括号文本、再按 `?` 高于 `|` 的优先级重解析，错误地产生了“union 的最后一个分支
nullable”。唯一 production owner 是 Skiff `compiler/source` 的 source type canonicalization，不应修改
OpenAI source，也不需要用户决定新的语言语义。

## 1. Checkpoint 与只读边界

| 对象 | 本次复现 checkpoint | tree |
| --- | --- | --- |
| Skiff audit worktree | `087ada637bd845a603734826fa4eb80c48138a56` | `330e09e244830b4bb8ed7bd2543b1fb013d08749` |
| skiff-packages phase-05 integration | `0ab4e7628b0a6aa90961c1485d2e58634b902676` | `5abb824e560778fd38a0a9a4e9936d189cc9f843` |
| `openai/` subtree | `2afcac70dbbbd611c0167c6f957f305ae0c9c9fa` | — |

- Skiff source与skiff-packages production全程只读；本result是唯一写入。
- fresh artifact root是
  `/tmp/skiff-p5-f379-openai-audit.3EANj6/artifacts`。提取证据后已删除整个616 KiB task-owned
  temp root，并确认路径不存在。
- 未连接或修改stable `4000/4001`、watch registry、stable artifact root、live service或外部OpenAI；
  publish在compile/contract validation阶段失败，没有发出provider请求。
- 复现后共享packages worktree由其他任务前进到`3653a294cfb92e60e220dcccc94bc8e8add65b33`；
  `0ab4…:openai`与`3653…:openai`的subtree hash均为`2afcac70…`，故该并发Registry改动不改变本次
  OpenAI证据。本文仍只把实际运行时的`0ab4…`声明为复现checkpoint。

## 2. Fresh canonical std 与精确复现

执行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f379-openai-nullable-union-audit/build/cargo-target \
cargo run --locked --quiet --manifest-path test-runner/Cargo.toml \
  --bin skiff-package-service-smoke-fixture -- \
  --bootstrap-only \
  --artifact-root /tmp/skiff-p5-f379-openai-audit.3EANj6/artifacts \
  --environment skiff-p5-f379 \
  --platform-source-root /Users/geek/workspace/skiff-p5-f379-openai-nullable-union-audit

CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f379-openai-nullable-union-audit/build/cargo-target \
node scripts/skiff.mjs package publish \
  /Users/geek/workspace/skiff-packages-phase-05-integration/openai \
  --artifact-root /tmp/skiff-p5-f379-openai-audit.3EANj6/artifacts \
  --environment dev --json
```

bootstrap **PASS**，并在fresh store写出：

| 对象 | canonical identity |
| --- | --- |
| std RuntimeAssembly | `skiff-runtime-assembly-v2:sha256:247fc2b3714bf715dc7918a10618be49493645efbbc0f293fc7b3d2e4d32b50f` |
| std PackageArtifact build | `skiff-package-build-v8:sha256:1828acdba6f3745db377255fc759fac3b3e87ed987001af97c67fa72bbbe4796` |
| std Package Local ABI | `skiff-package-local-abi-v6:sha256:c8be1d04060489a28f827a5313da12ae26891b1d3b21d1085b6e72884c9ab0ea` |

OpenAI publish在0.87秒内稳定 **FAIL**，完整相关诊断为：

```text
error: contract validation failed:
expression type model failed:
- openai: call `readImageResponse` argument 3 canonical type identity mismatch at 465:62: expected Local { local_type: Union { items: [Literal { value: String { value: "jpeg" } }, Literal { value: String { value: "png" } }, Nullable { inner: Literal { value: String { value: "webp" } } }] } }, found Nullable { inner: Local { local_type: Union { items: [Literal { value: String { value: "jpeg" } }, Literal { value: String { value: "png" } }, Literal { value: String { value: "webp" } }] } } }
- openai: call `readImageResponse` argument 3 type mismatch at 465:62: expected "png" | "jpeg" | "webp"?, found "jpeg" | "png" | "webp"?
- openai: call `readImageResponse` argument 3 canonical type identity mismatch at 471:58: expected Local { local_type: Union { items: [Literal { value: String { value: "jpeg" } }, Literal { value: String { value: "png" } }, Nullable { inner: Literal { value: String { value: "webp" } } }] } }, found Nullable { inner: Local { local_type: Union { items: [Literal { value: String { value: "jpeg" } }, Literal { value: String { value: "png" } }, Literal { value: String { value: "webp" } }] } } }
- openai: call `readImageResponse` argument 3 type mismatch at 471:58: expected "png" | "jpeg" | "webp"?, found "jpeg" | "png" | "webp"?
```

其中两个human-readable type text都缺少表示树形的括号，不能据此区分nullable位置；上面的canonical
IR明确证明expected与actual的树形不同。

## 3. Source → canonical 逐跳追踪

OpenAI只`import std`（`openai/openai.skiff:1`）；`OpenAiImageFormat`不是imported alias，而是同文件
local alias。`openai/api.yml:2`只把该local alias导出为public package symbol，不参与本地两次call解析。

| 跳 | source spelling / 位置 | resolved canonical type | nullable位置 | 结论 |
| --- | --- | --- | --- | --- |
| alias声明 | `alias OpenAiImageFormat = "png" \| "jpeg" \| "webp"`，`openai.skiff:13` | `Union["jpeg","png","webp"]`（排序不改变集合） | 无nullable | 透明三分支literal union，声明正确 |
| generate字段 | `outputFormat: OpenAiImageFormat?`，`:27` | `Nullable(Union["jpeg","png","webp"])` | union外 | 正确 |
| edit字段 | 同上，`:40` | `Nullable(Union["jpeg","png","webp"])` | union外 | 正确 |
| 两个actual | `input.outputFormat`，`:465,:471` | `Nullable(Local(Union["jpeg","png","webp"]))` | union外 | field projection保留声明的结构化IR |
| callee声明 | `requestedFormat: OpenAiImageFormat?`，`:315` | 本应为`Nullable(Union[...])`；实际成为`Union["jpeg","png",Nullable("webp")]` | **错误地进入union最后一支** | **首个identity分叉** |
| call检查 | `readImageResponse(..., input.outputFormat)`，`:465,:471` | expected为错误的inner-nullable union；actual为正确的outer-nullable union | 两棵树不同 | 两个caller只是首个把两条路径相遇的位置 |

具体compiler路径如下：

1. record field projection走
   `compiler/source/src/type_resolution_model/shape_assignability.rs:1174-1203`，优先恢复declared field
   reference；`resolve_constructor_target_resolved`在
   `compiler/source/src/type_resolution_model.rs:1316-1381`用结构化`resolve_type_expr`后再
   `expand_alias_type_ref`。因此actual保持
   `Nullable(Union["png","jpeg","webp"])`。
2. local callable signature由
   `compiler/source/src/expression_type_model.rs:4235-4361`保留AST source spelling；参数检查在
   `:3210-3224`调用`resolve_type_text("OpenAiImageFormat?")`。
3. `resolve_type_text`在`compiler/source/src/type_resolution_model.rs:625-638`先调用
   `expand_alias_text`，再把结果重新`TypeExpr::parse`。
4. `expand_alias_text`在`:5100-5122`用`map_named_types`把alias RHS作为一个**字符串name**塞回
   `Nullable`节点；`syntax/src/type_expr.rs:112`将其输出成无括号的
   `"png" | "jpeg" | "webp"?`。
5. 重解析先在`syntax/src/type_expr.rs:55-60`切顶层`|`，再处理后缀`?`，因此只把`"webp"`变成
   nullable。这里是唯一、可复现的第一分叉点；后面的canonical comparison只是准确暴露它。

`mimeFromOutputFormat(requestedFormat)`（`:129,:329`）没有另报错，是因为callee参数和caller局部参数
都来自同一条错误的signature resolution路径；两端“同错”。`imageFormatText(input.outputFormat)`
（`:227,:434`）位于`input.outputFormat != null` narrowing之后，actual已变成non-null union，各literal
仍可分别进入错误expected union的对应分支，所以也暂时通过。这些通过不证明其canonical type正确。

## 4. 语言语义与唯一owner

现有reference无需改变：

- `doc/reference/syntax.md:88-90`规定union是语义集合，`T?`等价于`T | null`，且`?`绑定强于`|`；
- `doc/reference/static-semantics.md:41-49`规定alias是透明缩写，在参数传递、字段检查与contract usage
  前按RHS展开，语义identity来自RHS。

因此：

```text
OpenAiImageFormat?
= ("png" | "jpeg" | "webp") | null
= "png" | "jpeg" | ("webp" | null)
```

三种写法的值集合都是`{"png","jpeg","webp",null}`。正确canonical source identity应统一为：

```text
Nullable(Union["jpeg","png","webp"])
```

contract层已有同一规则：
`artifact-identity/src/contract/normalization.rs:44-92,201-216,239-265`会flatten structural union、
收集任意层级的null/nullable、排序去重，最后把nullable统一提升到整个base union之外。反之，
source层`canonicalize_type_ref`（`compiler/source/src/type_resolution_model.rs:1821-1906`）当前只递归
改写symbol owner，不会flatten/hoist nullable union；`union_type_ir`（`:5299-5305`）也只排序去重。

`P5-F273-public-alias-expansion-result.md`已明确要求结构化alias IR，并特意记录“避免
`Nullable(Union)`被错误解析成`Union(..., Nullable(...))`”。其artifact ingest修复与现有
`artifact_descriptors_preserve_nested_records_arrays_aliases_and_literal_unions`测试确实保住了imported
artifact record field，但本地`resolve_type_text`仍在结构化展开前执行旧的text round-trip，留下了本次
local callable gap。

所以三选一结论唯一：

- **不是**OpenAI source应显式重排/解包；
- **不是**两个类型语义不同；
- **是**compiler必须让语义等价的nullable-union得到同一个source canonical identity，并且alias
  semantic expansion不能经过无precedence信息的format-to-text-to-parse round-trip。

唯一production owner为`compiler/source/src/type_resolution_model.rs`。`syntax/src/type_expr.rs`的
printer使问题可见，但让printer替一个被伪装成`Named`字符串的整棵alias RHS猜测precedence不是最小、
也不是可靠的语义修复；source spelling可继续用于诊断，semantic IR必须保持结构化。

## 5. skiff-packages影响面

全量扫描production `.skiff`只发现四个literal-union alias：

| package / alias | nullable使用 | 当前结果 |
| --- | --- | --- |
| OpenAI `OpenAiImageFormat` | 两个request field（`:27,:40`）、三个function参数（`:129,:227,:315`）、一个generic type argument（`:228`） | 受影响；两处unnarrowed field→local-call形成当前hard failure |
| OpenAI `OpenAiImageQuality` | 两个request field（`:25,:38`）、一个function参数（`:223`）、一个generic type argument（`:224`） | 同一latent compiler defect；现有调用在non-null narrowing后通过 |
| http-session `HttpSessionSource` | `session.skiff:13,:331`均非nullable | 不触发该nullable/union分叉 |
| Registry `RegistryErrorTag` | `model.skiff:107-116`只有非nullable record field | 不触发该nullable/union分叉 |

因此：

- 当前publish diagnostic精确限于`:465`和`:471`两个caller；
- production风险不只两行：OpenAI中format与quality的全部nullable alias signature/field都经过同一
  有缺陷的canonical路径，只是部分路径因narrowing或“两端同错”未报错；
- packages中未发现第二个跨package hard blocker，但compiler是语言级owner，未来任何
  `alias U = A | B`配合`U?`都可复现；
- 只改两个caller、改alias顺序、把field手工拆成literal union或在OpenAI中提前unwrap都会留下
  compiler defect，并违背透明alias规则。

## 6. 现有测试与覆盖缺口

本次只读执行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f379-openai-nullable-union-audit/build/cargo-target \
cargo test --locked -p skiff-compiler-source \
  aliases_expand_exactly_through_callbacks_and_nested_structural_types -- --nocapture

CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f379-openai-nullable-union-audit/build/cargo-target \
cargo test --locked -p skiff-compiler-source \
  artifact_descriptors_preserve_nested_records_arrays_aliases_and_literal_unions -- --nocapture
```

两条均为 **PASS（各1/1）**。它们没有证伪本次问题：

- 第一条覆盖union alias作为non-null callback参数，以及nullable nominal type嵌在container内；没有覆盖
  `Nullable(alias whose RHS is union)`；
- 第二条直接从artifact descriptor断言外层`Nullable(Union)`；没有经过本地callable
  `resolve_type_text`；
- `syntax/src/type_expr`测试覆盖一般parse/round-trip和atomic name mapping，没有覆盖mapping callback
  把低precedence union文本注入nullable的情形；
- 没有现有expression-type测试将nullable union alias的record field直接传给同类型local parameter。

## 7. 最小successor

不需要用户决策。建议一个Skiff implementation节点完成后，OpenAI publish revalidation即可回到父DAG。

最小文件：

| 类型 | 文件 | 必须完成 |
| --- | --- | --- |
| production | `compiler/source/src/type_resolution_model.rs` | semantic alias expansion保持`TypeExpr`/`TypeRefIr`结构，不先展开成字符串再重解析；source canonicalization对nested union/null/nullable作与contract一致的flatten、sort/dedup、outer-nullable规范化 |
| direct unit regression | 同文件的`#[cfg(test)]` | 断言`Format?`解析为且只为outer `Nullable(Union[3])`；断言`Format?`、`Format \| null`和union某分支nullable的等价值集得到同一canonical identity |
| end-to-end source regression | `compiler/source/src/expression_type_model.rs`的`#[cfg(test)]` | 精确覆盖record field→local callable parameter的正例与null不应被擦除的负例 |

不需要修改`openai/openai.skiff`、`openai/api.yml`、package manifest、
`artifact-identity/src/contract/normalization.rs`或现有语言reference。

正例：

```skiff
alias Format = "png" | "jpeg" | "webp"
type Request { format: Format? }

function consume(format: Format?) -> void {}

function run(input: Request) -> void {
  consume(input.format)
}
```

该程序必须通过，expected与actual都必须投影为outer `Nullable(Union[3])`。

负例：

```skiff
alias Format = "png" | "jpeg" | "webp"
type Request { format: Format? }

function consume(format: Format) -> void {}

function run(input: Request) -> void {
  consume(input.format)
}
```

该程序必须以argument 1 type mismatch失败，证明canonicalization没有丢掉`null`。还应保留一个非成员
literal（例如`"gif"`）不能赋给`Format?`的负例，证明union成员没有被扩大。

focused implementation gate：

```bash
CARGO_TARGET_DIR=<successor-worktree>/build/cargo-target \
cargo test --locked -p skiff-compiler-source nullable_union_alias -- --nocapture

CARGO_TARGET_DIR=<successor-worktree>/build/cargo-target \
cargo test --locked -p skiff-compiler-source
```

随后用全新task-owned artifact root重复本result第2节的canonical std bootstrap与OpenAI
`package publish`。验收条件是：

1. direct canonical与expression-type正负回归全部PASS；
2. OpenAI source零改动；
3. fresh OpenAI publish生成PackageArtifact receipt，不再出现`:465/:471` identity/type mismatch；
4. temp root清理，仍不访问stable/live/external OpenAI。

父节点F375的完整packages gate应在该focused修复验收后单独继续；本节点不把父节点记录的其它Registry/
Router事项纳入nullable-union successor。
