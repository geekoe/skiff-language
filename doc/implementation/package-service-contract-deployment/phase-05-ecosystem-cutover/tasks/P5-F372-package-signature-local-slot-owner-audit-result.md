# P5-F372 Package signature local-slot owner audit result

状态：Complete（只读审计；不需要用户决策）。

## 结论

`std.http.stream -> HttpClientStreamHandle`在source fact和FileIR中都仍有正确的
`std.http`模块上下文。owner第一次不可逆丢失发生在PackageArtifact public callable signature生成阶段：

1. `compiler/projection/src/package_artifact/callables/normalization.rs::
   normalize_local_type`已经收到callable的`owner_module = "std.http"`；
2. slot 7对应的`HttpClientStreamHandle`含`Stream<bytes>`，不满足PackageSchema boundary类型集合；
3. PackageSchema promotion miss后，现实现原样保留`LocalType(7)`；
4. `callables/mod.rs`把这个没有module owner的值写进
   `packageLocalAbi.publicSymbols["std.http.stream"].signature`。

同一个artifact的`implementationLinks.functions["std.http.stream"]`已经把该返回值正确写成：

```json
{
  "kind": "serviceSymbol",
  "symbol": {
    "modulePath": "std.http",
    "symbol": "HttpClientStreamHandle"
  }
}
```

consumer随后只能拿package和slot 7遍历所有module猜owner；std中slot 7同时对应
`std.http.HttpClientStreamHandle`和`std.websocket.WebSocketCloseEvent`，因而报ambiguous owners。

## 冻结的不变量

canonical owner是PackageArtifact producer：

- 写入`PackageLocalAbi.publicSymbols`前，公开callable参数和返回值的任意嵌套位置不得残留
  `TypeRefIr::LocalType`；
- schema-eligible nominal仍提升为`PackageSchema`；
- schema promotion miss的local/publication nominal必须依据现有module-slot映射转换为exact
  `ServiceSymbol { module_path, symbol }`；
- missing或ambiguous module-slot映射必须fail closed；
- consumer遇到任何残余ownerless `LocalType`必须将artifact视为producer-invalid，即使当前package中只有
  一个slot候选；不得再猜唯一module、public path或display name。

不新增artifact DTO/variant，不改变artifact schema version。`PublicationType(module, slot)`不是目标形式，
因为它仍把execution-local slot固化到公开ABI；现有publication-visible canonical form已经是
`ServiceSymbol`。

## 影响面

F368真实生成的std、http-session、track和llm-api artifact中只有一个raw `LocalType`：

```text
skiff.run/std
└─ packageLocalAbi.publicSymbols.std.http.stream.signature.returnType
```

当前失败虽然只有这一处，修复必须递归覆盖：

- 普通public function和public-instance operation；
- 所有parameters与return；
- `PackageTypeRef`的Local、Container、Nullable、AnyInterface；
- 内层`TypeRefIr`的Builtin、AppliedNominal、Record、Union、Nullable、Function和AnyInterface。

producer当前对内层`TypeRefIr::AnyInterface`走wildcard原样返回，是同一修复节点必须关闭的盲点。

## Identity与验证链

修复会改变std Package Local ABI identity、std build identity及official std build golden，但不改变：

- `skiff-package-artifact-v7`；
- Local ABI/build identity prefix；
- PackageSchema generation、index或type ID。

依赖旧std Local ABI的真实receipt必须按以下最小DAG重建：

```text
std
├── http-session ──> track
└── llm-api ───────> llm-providers
```

后三者自己的Local ABI预计不变，以fresh receipt为准。实现边界已经足够明确，后继节点为
`P5-F374-package-signature-exact-symbol-owner.md`。
