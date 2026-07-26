# P5-F353 Public generic Package API / schema availability

状态：Ready（C1 projection capability leaf）。

## 直接父节点

- `P5-H36-external-ingress-implementation-dag.md`
- `P5-F349-public-generic-boundary-availability-audit-result.md`

## DAG位置与目标

解除public generic declaration导致Package整体失败的first loss，同时保持PackageSchema与service boundary
strict。该leaf与F352、F354并行；不修改它们的authoring/artifact/gateway owner。

必须形成：

1. public generic type/representation/named union/interface可以进入PackageLocalAbi与implementation links；
2. generic declaration、free`TypeParam`、`AppliedNominal`或传递引用这些shape的non-generic owner整体
   schema-ineligible：不写index/ref/record，不产生partial/dangling closure，也不让Package失败；
3. eligibility递归检查被引用owner的`type_params`，不能只按public path找到definition就接受；
4. public callable的完整signature仍进入Local ABI；若参数/返回/stream/callback使用上述shape，
   `BoundaryCallableProjection`为已有`Unavailable(UnsupportedBoundaryType)`；
5. 删除PackageSchema/service-call projection中按`std.websocket`名字放行generic platform type的特例；
   source/prelude对generic声明本身的解析保留；
6. schema-closed public/error types继续生成精确records；strict dependency/schema/tamper admission不放松。

## Production owner与写入范围

允许修改：

- `compiler/projection/src/package_artifact/api_exports.rs`
- `compiler/projection/src/package_artifact/schema.rs`
- `compiler/projection/src/package_artifact/boundary/**`中仅用于移除generic/WebSocket service-boundary特例的
  最小代码
- 专用projection/compiler tests与必要fixture/golden

禁止修改：

- `compiler/input*`、`compiler/source/src/api_yml.rs`、`compiler/contract/src/projection.rs`；
- `artifact-model`/`artifact-identity` schema或generation；
- gateway/deployment/runtime/router/test-runner、std source、三仓库service、lockfile。

测试尽量新增专用文件，避免修改F352拥有的PackageArtifact构造器；若新字段使分支暂时无法编译，报告给主
Agent等待F352 checkpoint，不自行复制其字段设计。

## 验证

```bash
cargo test -p skiff-compiler-projection package_schema -- --list
cargo test -p skiff-compiler public_generic -- --list
cargo test -p skiff-compiler-projection package_schema
cargo test -p skiff-compiler public_generic
cargo test -p skiff-compiler package_imports
cargo fmt -p skiff-compiler-projection -p skiff-compiler -- --check
git diff --check
```

selector必须非零。必须证明：任意owner/name generic均Local ABI/link存在且schema为零；non-generic传递引用
generic/applied nominal同样整体缺席；无partial record；callable为structured Unavailable；wrong arity、
forged generic schema、record owner/key/closure篡改仍拒绝；一个普通schema-closed类型和
`std.service.InternalError`对照仍存在。

不运行workspace/root、stable/live，不push。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f353-generic-schema`
- branch：`codex/p5-f353-generic-schema`
- 从包含本task的integration checkpoint创建；result记录exact base/commit/tree。
- 提交production/tests，再提交result；worktree保持clean，不merge/rebase integration。
