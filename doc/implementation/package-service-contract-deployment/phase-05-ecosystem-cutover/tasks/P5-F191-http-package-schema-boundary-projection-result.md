# P5-F191：HTTP Package Schema Boundary 投影结果

状态：完成

## 结果

- compiler boundary 现在消费编译驱动已经按精确 PackageArtifact 校验的
  `ResolvedPackageSchema`；`PackageSymbol` 只有在依赖别名或 package id 唯一命中，并且目标是
  `api.yml` 公开且存在 schema record 的类型时，才投影为 `ContractTypeRef::PackageSchema`。
- `std.http.HttpRequest`、`std.http.HttpResponse` 和
  `std.http.HttpResponseStreamEvent` 不再依赖 builtin/native boundary 特例；原有 HTTP builtin
  admission 已删除。
- 未声明依赖、未知公开路径、重复 package-id 绑定和没有 schema record 的普通 Package
  record/interface 继续以 `UnsupportedBoundaryType` 失败关闭。

## 真实验证

在独立临时 artifact root 中先通过 canonical bootstrap 发布
`skiff.run/std@1.0.0`，再发布 `skiff.run/http-session@1.0.0`，随后以真实 Account 源码和仅保留
`account.ping` ingress 的临时 service/profile 投影：

- Account PackageArtifact：
  `skiff-package-build-v4:sha256:f85a78a7647c8c187541bebd4cf38a628fa8988b0e3ce0f266bebbd326c36a9f`
- Account ping request：
  `PackageSchema(skiff.run/std, std.http.HttpRequest)`
- Account ping return：
  `PackageSchema(skiff.run/std, std.http.HttpResponse)`
- ServiceProtocol：
  `skiff-service-protocol-v3:sha256:5db2fffe86ea2f6b330c840a004d9e0fa64443f38755706e7a42f2da26547288`
- Deployment：
  `skiff-deployment-artifact-v1:sha256:ff142a049177a018c97ee3e1ec358bbe30194d5d634aa2df823e69d373f8db8a`

临时 store 和临时 Account 副本已在验证后删除，未操作 stable。

## 测试

- `cargo test -p skiff-compiler-projection`：24/24 通过。
- `cargo test -p skiff-compiler --test prelude_std_schema imported_http_types_reach_unary_and_stream_boundary_projection -- --exact`：通过。
- `cargo test -p skiff-deployment`：48/48 通过。
- `cargo check --workspace`：通过。
- `cargo fmt --all`、`git diff --check`：通过。

补充运行完整 `prelude_std_schema` 时，既有
`stream_type_is_explicitly_boundary_unavailable` 仍失败；该测试把 `Stream<string>` 期望为
`UnsupportedStream`，与本任务的 Package schema 投影无关，未在 F191 中扩大修改范围。
