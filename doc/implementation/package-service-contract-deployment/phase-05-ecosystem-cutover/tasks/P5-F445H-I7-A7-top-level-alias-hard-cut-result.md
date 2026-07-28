# P5-F445H-I7-A7 topLevelAlias hard cut result

状态：

```text
COMPLETE
A7_COMPLETE=YES
ACCESS_TOP_LEVEL_REMOVED=YES
TOP_LEVEL_ALIAS_COMPLETE=YES
SECOND_REQUIREMENT_OR_PROJECTION=NO
SCHEMA_GENERATION_CHANGED=NO
DECISION_REQUIRED=NO
SCOPE_EXPANDED=NO
```

## 1. Result

package dependency现在只有一个公开alias，并可在test service中额外声明一个
`topLevelAlias`：

```yaml
packages:
  - id: example.com/subject
    version: 1.0.0
    alias: subject
    topLevelAlias: subjectImpl
```

`subject`只解析`api.yml`公开面，`subjectImpl`只解析该精确依赖的source top-level。
两者没有fallback或优先级关系，`topLevelAlias`也不会沿依赖图传递。

旧`access`字段及`PackageDependencyAccess`已经删除；manifest parser保持strict，
因此旧`access: topLevel`直接作为未知字段失败。`topLevelAlias`只允许出现在
`service.yml kind: test`对应package的package dependency上，并且必须是合法、非保留且在完整
package/service alias namespace中唯一的标识符。

## 2. One dependency, two authoring views

两个authoring alias仍对应同一条manifest dependency：

- source解析时由alias显式选择公开面或source top-level；
- lowering把top-level引用规范化回primary alias；
- File IR中的`dependency_ref`、expected local ABI与expected build均属于primary dependency；
- compiler只生成一个`PackageRequirement`；
- test assembly只生成一个binding、code slot与collection projection。

新增compiler integration fixture让公开面和source top-level具有相同文本symbol path，分别通过
primary alias和`topLevelAlias`访问，证明解析没有隐式fallback。test-runner receipt同时断言subject
只有一个requirement与一个binding/projection。

实现没有增加artifact DTO字段，没有修改schema/identity generation，也没有修改runtime或loader。
artifact model仅更新了既有字段注释，以说明authoring-local alias不会进入artifact。

## 3. RED to GREEN

在冻结baseline
`5e87d1ce3c3461e5687564807afea9db4943ba46` /
`c9481fc7859919199ac84e6839b07847779fce02`
上先加入`topLevelAlias` integration fixture，得到预期RED：

```text
unknown field `topLevelAlias`
```

实现后，同一fixture证明：

- primary alias仍只能读取public API；
- `topLevelAlias`可读取source top-level；
- 两个view共享一个带精确ABI/build约束的requirement；
- 通过top-level alias访问间接依赖失败；
- P4 canonical descriptor等价测试继续通过。

## 4. Verification

```text
cargo test -p skiff-compiler-input-model
PASS 3/3

cargo test -p skiff-compiler-input
PASS 100/100

cargo test -p skiff-compiler --test package_imports
PASS 12/12

cargo test -p skiff-compiler-lowering publication_local_refs
PASS 3/3

cargo test -p skiff-compiler --lib \
  authoring::tests::ordinary_package_authoring_rejects_top_level_alias_before_dependency_resolution -- --exact
PASS 1/1

cargo test -p skiff-compiler --lib \
  pipeline::tests::top_level_alias_keeps_one_primary_requirement_and_pins_the_exact_build -- --exact
PASS 1/1

cargo test -p skiff-test-runner --test package_service_contract_deployment
PASS 28/28, 1 ignored

cargo check -p skiff-compiler --tests --locked
PASS

cargo check -p skiff-test-runner --tests --locked
PASS

node --check scripts/tests/package-service-host-negative-probe.test.mjs
PASS

cargo fmt --all -- --check
PASS

git diff --check
PASS
```

完整`skiff-compiler-source`测试仍为`333/337`；4个失败是baseline已有的
reserved-validation/prelude registry测试。相关source check和本任务三个聚焦resolver测试均通过，
没有引入新的失败。

Skiff仓库内现有test authoring与fixtures已全部迁移；旧`access: topLevel`只保留在两条明确验证
strict parser拒绝旧字段的负例中。本任务没有访问stable、live、network、Mongo、OAuth或browser，
也没有push。
