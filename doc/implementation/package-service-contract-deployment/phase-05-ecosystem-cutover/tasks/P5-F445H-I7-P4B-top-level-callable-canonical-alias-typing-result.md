# P5-F445H-I7-P4B top-level callable canonical alias typing result

状态：

```text
COMPLETE
P4B_COMPLETE=YES
AGINE_COMPILE_UNBLOCKED=YES
DECISION_REQUIRED=NO
SCOPE_EXPANDED=NO
```

## 1. Identity

| 项 | 值 |
| --- | --- |
| baseline | `e1530c6a0bdbc7ee4bf6ef9094de7e9a965a3b9e` / `50fba533c5698f06d556274db94eb11f0e3d7be4` |
| implementation | `4126106d5cf526f8579b2095e4d5202ab920940c` / `981df6ba706ad17876212e4a62a974af97308cef` |
| Internals fixture | `8aaa281d78a6555bdbca6cb5a58d6124941ac649` / `b289796cc8dc0d1c1aab05445711aa8ae7f9d7df` |

## 2. Result

source dependency callable lookup现在同时返回：

- 由输入alias精确选择的callable；
- 该view所属同一direct dependency的canonical primary alias。

`topLevelAlias`仍是选择private implementation callable的唯一入口，但表达式类型检查随后只使用
primary alias重建参数、返回值与exact projection。因此provider签名中的owner-local public type会绑定为
primary dependency身份，并与同一package通过`api.yml`公开的`PackageSchema`精确相等。

参数与返回值继续经过既有递归重建，所以container、nullable、applied nominal、interface和
`any Interface`没有新增旁路。实现没有引入结构等价、fallback或transitive top-level访问。

`SourceDependencyAnalysisInput`同时新增fail-closed检查：canonical primary view必须存在，不能指向另一
个view，并且source view与primary view必须选择相同的package build和Local ABI。PackageSchema owner、
stable key与type id仍通过既有exact lookup判定；错误owner、key或type id均不命中。

没有修改lowering、linker、runtime、artifact DTO/schema、manifest或identity代际。

## 3. RED to GREEN

新增真实package graph fixture：

- test consumer同一dependency声明`alias: provider`与`topLevelAlias: providerImpl`；
- private top-level callable参数及返回引用provider公开record；
- 参数同时覆盖`Array<PublicInput?>`、`any PublicHandler`；
- generic `PublicEnvelope<PublicInput>`覆盖applied nominal；
- 返回值赋给primary alias的`provider.PublicOutput`。

baseline RED精确为：

```text
expected ... Dependency { dependency_ref: "providerImpl" } ... PublicInput
found PackageSchema ... stable_schema_key: "PublicInput"
```

同类错误同时出现在container参数和公开返回值。修复后正例通过；private implementation type冒充
public type仍报告参数类型不匹配，primary alias调用private callable仍报告缺少public path。

## 4. Verification

```text
cargo test -p skiff-compiler-source dependency_analysis::tests --locked -- --nocapture
PASS 9/9

cargo test -p skiff-compiler --test package_imports --locked --no-fail-fast
PASS 14/14

cargo check -p skiff-compiler --tests --locked
PASS

cargo fmt --all -- --check
PASS

git diff --check
PASS
```

完整`skiff-compiler-source`为`337/341`。4个失败与baseline相同，仍是
reserved-validation越界、两个prelude identity snapshot及builtin spelling owner测试；P4B新增的两个
dependency tests均通过。

## 5. Internals Agine exact compile

使用冻结Internals tree、当前Skiff implementation及官方packages candidate，构建真实canonical图：

```text
packages=7
dependency services=2
assembly=skiff-runtime-assembly-v3:sha256:46716cfc193dab48b2af104553df576073728ef640f3d4081254a0b169eef07d
```

随后直接调用当前`skiff-test-runner`编译
`agine/service-tests`，提供上述artifact root与base assembly，但刻意不提供隔离Runtime artifact root。
编译已越过M3记录的`UserMessageInput`、`RunReceipt`和`ToolResult`同类canonical identity错误，最终明确
停在执行前边界：

```text
non-live tests require a harness-owned runtime artifact root outside --artifact-root
```

该探针只证明真实test service完整source compile已解阻，不宣称运行170个case。

首次尝试完整isolated harness时，其独立临时Cargo target在Runtime构建阶段耗尽磁盘，harness的临时
Mongo supervisor也随构建失败退出；该次运行不作为证据。上面的替代探针复用已构建target，只执行
authoring、assembly与compiler，没有启动Runtime、可用Mongo服务或网络流量。没有访问stable instance、
OAuth、browser或push。
