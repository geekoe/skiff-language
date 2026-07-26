# P5-F383 Structured nullable-union alias normalization result

状态：**Completed**。`compiler/source`现在让原始结构化`TypeExpr`直接进入semantic
resolution，再以结构化`TypeRefIr`展开透明alias；诊断文本不再参与semantic identity。
nested union/null/nullable统一flatten、排序、去重，并把nullable提升到完整base union外。
direct source回归和完整compiler/source套件通过，fresh canonical std上的OpenAI真实本地publish
产生`PackageArtifact` receipt，原`:465/:471` identity/type mismatch消失。

## 1. Exact checkpoint与边界

| 项目 | commit | tree |
| --- | --- | --- |
| task base | `b904dcda4f1b83ab94e99ab0dbf25f628015b605` | `439e08c5a1984ba2b399c8989926b060f17ca9f3` |
| production/tests | `2ec4f56372d8814263af5b930814c9d390910c12` | `9e5f4fc8b7a10177172c87c0c9e6a3378c2c3125` |

- worktree：
  `/Users/geek/workspace/skiff-p5-f383-nullable-union-alias`；
- branch：`codex/p5-f383-nullable-union-alias`；
- production只修改
  `compiler/source/src/type_resolution_model.rs`；
- direct source tests只修改
  `compiler/source/src/type_resolution_model.rs`与
  `compiler/source/src/expression_type_model.rs`中的`#[cfg(test)]`；
- 没有修改syntax precedence/printer、artifact contract normalization、artifact DTO/identity
  schema、语言reference、OpenAI source或其它仓库；
- 没有merge、rebase、push，也没有访问stable、live或外部OpenAI。

## 2. Structured semantic owner

`TypeResolutionModel::resolve_type_text`现在：

1. 从原始source spelling解析一次`TypeExpr`；
2. 独立计算alias-expanded `source_text`，只供诊断和既有generic-alias合法性检查；
3. 从原始结构化`TypeExpr`解析`TypeRefIr`；
4. 在IR树上递归展开alias。

因此`Format?`不再把alias RHS伪装成一个包含`|`的字符串name后重新format/reparse。
反向检查确认semantic路径中已不存在`TypeExpr::parse(&expanded)`。

`normalize_source_type_ref`成为source结构规范化owner，并递归覆盖builtin arguments、
applied nominal arguments、records、functions与interface type arguments。对union lane：

- flatten任意层级`Union`；
- 收集任意层级`Nullable`、builtin `null`和null literal；
- 以既有source type debug key排序并按结构去重；
- 单一非null成员折叠为该成员；
- 有null时只生成一个最外层`Nullable(base)`。

`expand_alias_type_ref`、assignability canonicalization、
module-owned canonicalization和既有`union_type_ir`都消费同一规范化owner，没有新增artifact
normalizer或syntax printer分支。

## 3. Direct regression

新增direct canonical test精确断言：

```text
Format?
Format | null
"png" | "jpeg" | "webp"?
null | "webp" | "png" | "jpeg" | "png"
```

全部得到：

```text
Nullable(
  Union[
    Literal("jpeg"),
    Literal("png"),
    Literal("webp")
  ]
)
```

重排并重复的non-null union稳定得到排序、去重后的同一三成员`Union`。

新增end-to-end expression-type tests覆盖：

- `Request.format: Format?`直接传给`consume(format: Format?)`通过；
- 同一field传给`consume(format: Format)`以`argument 1 type mismatch`失败，证明null未丢失；
- `"gif"`传给`Format?`参数以`argument 1 type mismatch`失败，证明成员集合未扩大。

既有alias、artifact descriptor、non-null union和其它compiler/source回归由完整套件继续覆盖。

## 4. 验证证据

| 层级 | 命令 | 结果 |
| --- | --- | --- |
| format | `cargo fmt --all -- --check` | PASS |
| focused | `CARGO_TARGET_DIR=<worktree>/build/cargo-target cargo test --locked -p skiff-compiler-source nullable_union_alias -- --nocapture` | PASS，4/4，315 filtered out |
| complete component | `CARGO_TARGET_DIR=<worktree>/build/cargo-target cargo test --locked -p skiff-compiler-source` | PASS，319/319；doctest 0；仅既有unused/dead-code warnings |
| patch hygiene | `git diff --check` | PASS |

最终production/tests commit上的focused与complete component证据均覆盖本次semantic owner；
没有运行workspace/root、live或父F375的完整packages gate。

## 5. Fresh std与OpenAI真实publish

task-owned fresh root为：

```text
/tmp/skiff-p5-f383-nullable-union-alias.vp8TdL/artifacts
```

bootstrap命令：

```bash
CARGO_TARGET_DIR=<worktree>/build/cargo-target \
cargo run --locked --quiet --manifest-path test-runner/Cargo.toml \
  --bin skiff-package-service-smoke-fixture -- \
  --bootstrap-only \
  --artifact-root /tmp/skiff-p5-f383-nullable-union-alias.vp8TdL/artifacts \
  --environment skiff-p5-f383 \
  --platform-source-root <worktree>
```

结果：PASS。fresh canonical std identities为：

| 对象 | identity |
| --- | --- |
| RuntimeAssembly | `skiff-runtime-assembly-v2:sha256:247fc2b3714bf715dc7918a10618be49493645efbbc0f293fc7b3d2e4d32b50f` |
| Package build | `skiff-package-build-v8:sha256:1828acdba6f3745db377255fc759fac3b3e87ed987001af97c67fa72bbbe4796` |
| Package Local ABI | `skiff-package-local-abi-v6:sha256:c8be1d04060489a28f827a5313da12ae26891b1d3b21d1085b6e72884c9ab0ea` |

真实本地publish命令：

```bash
CARGO_TARGET_DIR=<worktree>/build/cargo-target \
node scripts/skiff.mjs package publish \
  /Users/geek/workspace/skiff-packages-phase-05-integration/openai \
  --artifact-root /tmp/skiff-p5-f383-nullable-union-alias.vp8TdL/artifacts \
  --environment dev --json
```

结果：PASS并产生`packageArtifactReceipt`：

| 对象 | identity / path |
| --- | --- |
| OpenAI Package build | `skiff-package-build-v8:sha256:1d476d64e8e89e87959538dec91bd145eecb21fbaa3aeb65fa9f8924faf00b50` |
| OpenAI Package Local ABI | `skiff-package-local-abi-v6:sha256:e1aa058a97669a1eda014dce6bebeb95f47a75cfa980068b491e194d351dfd98` |
| PackageArtifact record | `records/package-artifacts/skiff~drun~sopenai/1.0.0/1d476d64e8e89e87959538dec91bd145eecb21fbaa3aeb65fa9f8924faf00b50/package.json` |
| FileIr identity | `skiff-file-ir-v8:sha256:a15c1bd988b8505ae54cfcdcaffbd594fe7e982d940598112d521fed5be17f6e` |

fresh FileIr结构探针确认：

- `OpenAiImageFormat` alias target是排序后的三literal `Union`；
- `OpenAiImageGenerateRequest.outputFormat`与
  `OpenAiImageEditRequest.outputFormat`均为
  `Nullable(Union["jpeg","png","webp"])`；
- `readImageResponse.requestedFormat`也为完全相同的outer-nullable结构。

相对F379 baseline，callee expected从错误的
`Union["jpeg","png",Nullable("webp")]`变为
`Nullable(Union["jpeg","png","webp"])`，与两个field projection actual一致。publish成功且输出中
不再出现`:465/:471` identity/type mismatch。

OpenAI package checkpoint为
`3653a294cfb92e60e220dcccc94bc8e8add65b33`，publish前后`openai/` subtree均为
`2afcac70dbbbd611c0167c6f957f305ae0c9c9fa`，工作区无`openai/`改动。生成的884 KiB
task-owned temp root随后完整删除，并确认路径不存在。

## 6. 自验收矩阵

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| semantic alias expansion保持结构化 | `resolve_type_text`从原始`TypeExpr`解析IR后调用`expand_alias_type_ref` | semantic路径无`TypeExpr::parse(&expanded)`；expanded text只写`source_text` | focused canonical + E2E 4/4 |
| nullable/union canonical identity统一 | `normalize_source_type_ref`、`normalize_source_union`、`collect_source_union_member` | 无第二个新source union normalizer；artifact normalizer未改 | 等价写法、排序去重direct assertions；component 319/319 |
| null与成员集合fail closed | expression-type两个负例 | 无OpenAI source workaround或caller unwrap | non-null parameter与`"gif"`均报argument mismatch |
| OpenAI真实publish闭合 | fresh std bootstrap；PackageArtifact receipt；FileIr结构探针 | OpenAI subtree hash前后一致；`:465/:471` mismatch不再出现 | bootstrap PASS；publish PASS |
| 禁止表面不变 | diff只含两个compiler/source文件和本result | syntax、artifact schema/reference、OpenAI、stable/live均零改动 | `git diff --check` PASS |
