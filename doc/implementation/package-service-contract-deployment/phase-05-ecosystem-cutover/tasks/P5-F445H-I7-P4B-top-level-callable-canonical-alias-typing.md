# P5-F445H-I7-P4B top-level callable canonical alias typing

状态：

```text
IN_PROGRESS
```

## 1. Frozen inputs

| 项 | 值 |
| --- | --- |
| Skiff baseline | `e1530c6a0bdbc7ee4bf6ef9094de7e9a965a3b9e` / `50fba533c5698f06d556274db94eb11f0e3d7be4` |
| Internals fixture | `8aaa281d78a6555bdbca6cb5a58d6124941ac649` / `b289796cc8dc0d1c1aab05445711aa8ae7f9d7df` |
| parent results | P4、P3C、A7、M3 |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-p4b-package-schema-fix` |
| branch | `codex/p5-f445h-i7-p4b-package-schema-fix` |

## 2. Problem

`package_callable_by_source_path`使用`topLevelAlias`正确选择implementation callable，却只把callable
返回给表达式类型检查。调用方随后错误地复用输入路径中的`topLevelAlias`重建签名，使provider-owned
local type绑定到implementation view，不能与同一direct dependency通过primary alias公开的精确
`PackageSchema`类型相等。

## 3. Required closure

- callable source-path lookup同时返回同一direct dependency的canonical primary alias；
- implementation callable仍只由`topLevelAlias`选择，普通alias不得访问private callable；
- 参数、返回值、container、nullable、applied nominal、interface与`any Interface`签名都使用
  canonical primary alias重建、绑定和记录exact projection；
- canonical alias必须存在，并与view的exact package build及Local ABI一致；
- PackageSchema stable key/type id、implementation link与artifact身份继续精确验证，不增加结构猜测、
  宽松相等、fallback或transitive top-level访问。

## 4. Scope

允许修改：

- `compiler/source/src/dependency_analysis.rs`及就地tests；
- `compiler/source/src/expression_type_model.rs`及就地tests；
- `compiler/tests/package_imports.rs`；
- 本task/result文档。

禁止修改P3 lowering/link/runtime、artifact DTO/schema、manifest、identity代际或Internals source。
若实现需要跨越上述边界，停止并报告scope expansion。

## 5. Acceptance

1. 真实RED→GREEN覆盖public PackageSchema参数、返回赋值、nested/container/nullable/interface；
2. private implementation type不能伪装public type；
3. 缺失或错误canonical alias、build、Local ABI、stable key/type id继续fail closed；
4. primary alias不能调用private implementation callable，top-level访问不传递；
5. P3C与现有package import矩阵不回归；
6. Internals Agine exact compile越过`UserMessageInput`、`RunReceipt`、`ToolResult`同类错误并到达明确后续阶段；
7. compiler source/package imports、check、fmt与diff gate通过。
