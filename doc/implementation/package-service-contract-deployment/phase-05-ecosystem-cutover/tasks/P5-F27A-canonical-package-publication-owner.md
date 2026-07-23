# P5-F27A：Canonical Package Publication / Official Std Authoring Owner

依赖D38 complete。独占compiler driver authoring/publication模块及直接tests，不改test-runner、scripts、runtime/Router或
user language semantics。独立worktree/branch，一个clean commit。

抽出唯一typed `PublishedPackageArtifact` publication writer，统一FileIR/resources/PackageArtifact canonical records与exact
record paths；top-level和nested `artifactPath`字节只有此owner决定。新增仅从validated `CompilerPlatformSources`派生manifest/
sources/prelude的official std authoring route；不能接收任意std root，不能放宽user `read_user_package_manifest` reserved-id与
official-source guard。返回typed publication receipt供下游seed消费，不在compiler内更新environment。

正反tests：固定std/prelude/build identity、writer byte determinism、same candidate重复、非授权copied std零写、user reserved
继续拒绝、wrong platform identity/manifest/source fail closed。只跑compiler authoring/publication精确tests、DAG/check/fmt/
diff-check；禁止fixture Cargo/smoke/full/I16/Host/stable。
