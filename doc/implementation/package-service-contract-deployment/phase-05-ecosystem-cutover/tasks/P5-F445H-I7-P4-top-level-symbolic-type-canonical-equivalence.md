# P5-F445H-I7-P4 top-level symbolic type canonical equivalence

状态：

```text
READY
```

## 1. Frozen inputs

| 项 | 值 |
| --- | --- |
| Skiff baseline | `e26fb9e39fa9dcfcbd22fb59acafd18428557d03` / `a901233137b18d402dc7bd9675755caea0323c42` |
| Internals M fixture | `7fa2ac5de5a576013ee2be74032435a361c8a6e4` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-p4-symbolic-type-fix` |
| branch | `codex/p5-f445h-i7-p4-symbolic-type-fix` |

## 2. Problem

当一个package type的public path与source path相同时，`implementation_links.types`与
`package_local_abi.implementation_symbols`可以分别用`ServiceSymbol`和`PackageSymbol`描述同一个
嵌套类型。`artifact_symbolic_type_index`当前直接比较两份descriptor字节形态，把合法的同一类型身份
误判为不一致。`any Interface`的`interface_abi_id`中也可能嵌套同一类等价表示。

## 3. Required closure

- 使用现有`PackageTypeSymbolIndex`和`normalize_package_interface_type_ref`，在比较前把两份
  descriptor规范化到同一package-owned identity；
- interface methods使用现有`normalize_package_interface_method_signatures`；
- `topLevel`选择时以`implementation_symbols`路径为规范化authority，不能被API别名抢占；
- `file/module/type_index`、`isInterface`、`typeParams`、nested package/path/ABI与interface method
  仍须精确一致；
- public依赖仍只见`api.yml`，`topLevel`依赖仍只见source top-level surface。

## 4. Scope

允许修改：

- `compiler/source/src/type_resolution_model.rs`；
- 该文件就地tests；
- `compiler/tests/package_imports.rs`；
- 本task/result文档。

禁止修改projection、artifact model、identity、linker、runtime、schema代际与Internals source。

## 5. Acceptance

1. 正例覆盖public path等于source path、字段引用同package另一类型、`any Interface`嵌套身份，以及
   test package同时以`topLevel`依赖subject和agent；
2. 篡改interface method、nested package/path/ABI和file/type slot继续fail closed；
3. Internals M Agine真实隔离编译越过`artifact_symbolic_type_index`，首错后移到已知foreign DB
   operation lowering缺口；
4. 聚焦tests、check、fmt与diff gate通过。
