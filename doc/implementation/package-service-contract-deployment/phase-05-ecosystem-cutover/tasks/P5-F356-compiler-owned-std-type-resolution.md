# P5-F356 Compiler-owned std type-resolution owner

状态：Ready（独立source-owner follow-up；由F353有界探查发现）。

## 直接父节点

- `P5-F353-public-generic-schema-availability-result.md`
- `P5-F352-service-call-root-selection-result.md`

父节点沿引用链连接唯一权威设计。本任务只修复compiler-owned `std` exact callable signature在source
type resolution中的owner接线，不改变Package/Service/Gateway语义。

## 已确认问题

`compiler/driver/source_compile/canonical_dependencies.rs`会从`available_packages`中选择并验证唯一canonical
`skiff.run/std`，并把它作为compiler-owned alias `std`写入`SourceDependencyAnalysisInput`。但
`compiler/driver/source_compile/mod.rs::build`只把`input.dependency_packages`传给
`CompileParsedPackageSourcesInput.package_artifacts`。

因此表达式类型分析拿到`std.*` callable的exact `PackageCallableSignature`后，调用
`TypeResolutionModel::rehydrate_package_signature_type_for_dependency("std", ...)`时找不到package owner，
报：

```text
dependency alias `std` has no resolved package owner
```

已知真实触发包括：一个声明service contract dependency的package函数体调用
`std.time.sleep`或`std.websocket.sendTextToConnection`。

## 目标

让compiler-owned `std`在source dependency analysis、exact callable signature rehydration、type slot/
Local ABI owner与后续lowering中使用同一个canonical PackageArtifact身份，同时保持：

- `std`不需要写入用户`package.yml`；
- 任意其它`available_packages`不能因此进入source name/type resolution；
- service contract dependency仍只提供code-free ServiceContract/PackageSchema，不泄漏provider package；
- missing、duplicate、wrong identity或ambiguous compiler-owned std继续fail closed。

## 必须完成

1. 建立一个canonical owner接线：
   - source dependency analysis选择的compiler-owned `std` exact artifact；
   - `TypeResolutionModel`用于rehydrate std signature的package id、Local ABI、build与type slot owner；
   - lowering输出中std package refs的expected identity；
   三者必须来自同一已验证artifact，不能分别重选或按alias猜。
2. 不得把整个`available_packages`直接当作普通`dependency_packages`：
   - 只允许已经由compiler-owned std选择规则接纳的artifact参与source type resolution；
   - undeclared非std available package不能获得alias、callable、type或identity owner。
3. 不得要求用户声明`std` package dependency，也不得给`package.yml`、`api.yml`或contract dependency增加
   兼容字段。
4. 保持F353语义：
   - generic std declaration可以保留Package Local ABI/link并在不适合PackageSchema/service boundary时
     structured unavailable；
   - 不恢复任何`std.websocket`名称特例；
   - exact callable type参数、AppliedNominal及Local ABI owner不能被擦除。
5. 删除或合并因修复出现的重复std artifact选择逻辑；canonical选择/identity validation只能有一个明确owner。

若有界实现调查证明需要改变`PackageCompileInput.available_packages`的公共职责、service contract input或
PackageArtifact schema，立即按工作流返回`TASK_SCOPE_EXPANDED`，不要自行扩大。

## 写入范围

允许：

- `compiler/driver/source_compile/**`；
- `compiler/source`中type-resolution owner接线及直接tests；
- 必要的compiler input DTO内部字段，但不得改变authoring/public wire；
- `compiler/tests`中的最小真实回归fixture。

禁止：

- F353 PackageSchema eligibility规则；
- ServiceContract、PackageArtifact、gateway/deployment/runtime/router/test-runner语义；
- 三仓库service源码、stable/live配置、lockfile。

## 验证

先列出并确认selector非零，再运行：

```bash
cargo test -p skiff-compiler-source compiler_owned -- --list
cargo test -p skiff-compiler compiler_owned_std -- --list
cargo test -p skiff-compiler-source compiler_owned
cargo test -p skiff-compiler compiler_owned_std
cargo test -p skiff-compiler public_generic
cargo test -p skiff-compiler service_call
cargo check -p skiff-compiler-source -p skiff-compiler
git diff --check
```

必须覆盖：

- service contract dependency与`std.time.sleep`同在一个真实package；
- service contract dependency与`std.websocket.sendTextToConnection`同在一个真实package；
- exact std generic parameter/return owner可以rehydrate并lower；
- duplicate std、wrong identity或缺失required std fail closed；
- undeclared非std available package不能被source解析；
- F353 public generic与F352 service-call focused selector不回归。

不运行workspace/root、stable/live，不push。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f356-std-owner`
- branch：`codex/p5-f356-std-owner`
- 从包含本task的integration checkpoint创建；
- 提交production/tests，再提交result；
- result记录exact base/commit/tree、真实回归路径和negative owner证据；
- worktree保持clean，不merge/rebase integration。
