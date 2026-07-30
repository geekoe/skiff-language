# P5-F374 Package signature exact symbol owner result

状态：Complete。

直接父节点：`P5-F372-package-signature-local-slot-owner-audit-result.md`。

## 实现结果

- `PackageArtifact` callable producer现在先用callable真实source module和对应File IR
  `type_table`解析module-local slot。`PackageSchema` promotion保持优先；promotion miss时只有精确
  linkable symbol才能写成`ServiceSymbol { module_path, symbol }`。
- missing module、missing slot、missing/nonexported symbol、重复module映射和错误link target slot都在
  projection阶段fail closed。实现不读取public/display path，也不新增artifact字段、variant或版本。
- `PackageTypeRef::{Local, Container, Nullable, AnyInterface}`以及内层
  `Builtin`、`AppliedNominal`、`Record`、`Union`、`Nullable`、`Function`和`AnyInterface`
  全部递归规范化。内层`AnyInterface.interfaceAbiId`会先解码canonical `TypeRefIr`，规范化后再写回
  canonical ABI key；canonical type arguments使用同一递归规则。
- consumer删除了跨package module按slot寻找唯一owner的fallback。残余raw
  `TypeRefIr::LocalType`无论零、一个还是多个候选都返回producer-invalid错误；
  `PublicationType`、`ServiceSymbol`、`PackageSymbol`和`PackageSchema`路径保持有效。
- official std build golden更新为fresh deterministic build identity；artifact schema/version、identity
  prefix和PackageSchema index identity均未改变。

## 聚焦覆盖

Producer单测覆盖：

- direct parameter/return；
- 外层`Container`、`Nullable`、`AnyInterface`；
- 内层`Builtin`、`Record`、`Union`、`Function`、`AppliedNominal`、`Nullable`和
  `AnyInterface` identity/type arguments；
- schema-eligible nominal继续提升为`PackageSchema`；
- source module与public/display path不同，且`PublicationType`使用其显式module owner；
- missing module/slot/symbol、private/nonexported symbol、错误owner slot和重复module映射失败关闭。

Consumer单测覆盖raw `LocalType`单候选和双候选均拒绝，同时保留owner-safe嵌套路径。
真实compiler probe从fresh official std读取`std.http.stream`，并实际编译、lower一个调用该API的package。

## Fresh isolated publication receipts

最终验证使用：

- Skiff：本任务worktree；
- official package source：
  `/Users/geek/workspace/skiff-packages-phase-05-integration`，
  commit `0ab4e762`；
- Internals package source：
  `/Users/geek/workspace/internals-phase-05-integration`，
  commit `3a72346`；
- isolated artifact root：
  `/tmp/skiff-p5-f374-final.Fns807/ecosystem-store`（记录证据后删除）。

未读取或修改stable artifact root、stable instance或live状态。

| package | fresh build identity | fresh Local ABI | F368对比 |
| --- | --- | --- | --- |
| `skiff.run/std@1.0.0` | `skiff-package-build-v8:sha256:1828acdba6f3745db377255fc759fac3b3e87ed987001af97c67fa72bbbe4796` | `skiff-package-local-abi-v6:sha256:c8be1d04060489a28f827a5313da12ae26891b1d3b21d1085b6e72884c9ab0ea` | build与Local ABI按预期改变 |
| `skiff.run/http-session@1.0.0` | `skiff-package-build-v8:sha256:1efd5bb697830286333438b0a9ac8b16b7121f0678c6827b3a71ce47cc1f068b` | `skiff-package-local-abi-v6:sha256:a9531023bdd44b10b87fe2c88dc0fb695a62e1a2d667a6696212cab31182009f` | Local ABI与F368完全相同 |
| `skiff.run/track@1.0.0` | `skiff-package-build-v8:sha256:161c9c2a403a881d8b9daa8b0983e6889d6602a59ff3cd6aadeff1be0d94554d` | `skiff-package-local-abi-v6:sha256:70b1097f3be8bc2b8da95e19aaa8eb48c38d971b4144071096a2e4a44e581d57` | Local ABI与F368完全相同 |
| `agine.ai/llm-api@0.1.0` | `skiff-package-build-v8:sha256:e8cf8c47784a5ad73e787dd00932340557d1152df2686925a66caefb9223b89c` | `skiff-package-local-abi-v6:sha256:2ee60ec2ac1519682fe26672cf944a2a980a5276a90a4aca150bd00be7afaf7d` | Local ABI与F368完全相同 |
| `agine.ai/llm-providers@0.1.0` | `skiff-package-build-v8:sha256:08e9d0f4629a86a3e5d4f8c164ad61a6043ad3c1da074b74a276af3a9575ad19` | `skiff-package-local-abi-v6:sha256:b2e0c75288e29275123c20df2afcef058af2d38047634f13567ad0d1bb3f2057` | F368在slot 7歧义处停止；这是首个有效receipt |

所有可比较的非std Local ABI均保持不变（3/3）。这些package的build identity改变是因为
`packageRequirements.expectedLocalAbi`现在引用fresh std ABI；不是其自身public Local ABI变化。

Fresh std仍使用
`skiff-package-schema-index-v1:sha256:1f70d5626cddaab23d51d52db974a9292cf019cb0161d67ff560c599ed6fd7fe`，
与F368相同。最终artifact结构探针确认：

```json
{
  "std.http.stream.returnType": {
    "kind": "local",
    "localType": {
      "kind": "serviceSymbol",
      "symbol": {
        "modulePath": "std.http",
        "symbol": "HttpClientStreamHandle"
      }
    }
  },
  "freshStdPublicSymbolsRawLocalTypeCount": 0
}
```

五个fresh artifact的`packageLocalAbi.publicSymbols`递归raw `LocalType`计数均为零。
`llm-providers`完整publish成功，不再出现slot 7 ambiguous owners。

## 验证

通过：

```text
cargo test -p skiff-compiler-projection \
  package_artifact::callables::normalization::tests
  -> 10 passed

cargo test -p skiff-compiler-source package_signature
  -> 2 passed

cargo test -p skiff-compiler --test compiler_owned_std_type_resolution std_http_stream
  -> 1 passed

cargo test -p skiff-compiler --lib \
  official_std_authoring_and_record_writer_are_fixed_and_deterministic
  -> 1 passed

cargo test -p skiff-artifact-identity \
  callable_parameter_return_and_suspend_mutations_change_local_abi_without_throw_set
  -> 1 passed

cargo check -p skiff-compiler-projection -p skiff-compiler-source -p skiff-compiler
  -> passed

git diff --check
  -> passed
```

任务文件中的不带`--lib`命令：

```text
cargo test -p skiff-compiler \
  official_std_authoring_and_record_writer_are_fixed_and_deterministic
```

会在选择/运行目标单测前编译全部integration test，并被本任务基线已有的
`compiler/tests/actor_dispatch_linking.rs:92`阻断：该fixture仍初始化已删除的
`RuntimeAssembly.global_ingress`字段（当前字段为`gateway_ingress`）。本任务未修改该文件或
RuntimeAssembly owner；同一official std测试以精确`--lib` target执行通过。

## 自验收矩阵

| 任务条款 | 代码证据 | 反向搜索证据 | 测试/真实证据 |
| --- | --- | --- | --- |
| producer exact owner | `normalize_local_type`、`exact_type_symbol`、`exact_public_type_symbol` | promotion miss不再返回原始`LocalType`；不读取public path | normalization 10 tests；fresh std exact return |
| 全递归形状 | `normalize_package_type`、`normalize_local_type`的完整递归match | 内层`AnyInterface`不再走wildcard clone | nested shape test；fresh std raw count 0 |
| consumer fail closed | `rehydrate_package_signature_local_type`的raw `LocalType`分支 | package-global candidate search和`ambiguous owners`fallback已删除 | source package-signature 2 tests |
| schema与identity稳定边界 | promotion优先；无DTO/schema/prefix改动 | schema index identity与F368相同 | identity mutation test；official std deterministic test |
| 真实最小DAG | isolated `std -> {http-session -> track, llm-api -> llm-providers}` | 五个publicSymbols raw count均为0 | 五个fresh receipt；llm-providers publish成功 |

没有merge、rebase或push；没有操作stable/live。
