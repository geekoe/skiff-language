# P2-R10G：Shared Fixture File-write Owner

状态：independent review abstraction repair；依赖 R10B/R10C/R10E，R10F 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“Compiler 与 Projection 流水线”。

## 目标与 ownership

- `TestDir` 成为 integration fixtures 唯一的“解析相对路径、创建父目录、写入 bytes” owner。
- 独占 `compiler/tests/common/test_dir.rs` 与以下重复 consumer call sites：
  `artifact_model_conformance.rs`、`config_shape.rs`、`db_process_metadata.rs`、`package_std_schema.rs`、
  `prelude_std_schema.rs`、`provider_connect_packages.rs`、`publication_resources.rs`、
  `test_artifact_identity.rs`、`package_unit_single_path.rs`、`shared_fixture_lane_probes.rs`。
- 禁止修改 production、compile/dependency helpers、其它 tests 或测试语义。

## 完成态

1. `TestDir` 提供一个适用于文本/bytes 的 file-write API，统一 parent creation 与 IO error context。
2. 上述 consumers 删除本地 `write_file`/等价 helper 和重复 `create_dir_all + fs::write` 模式。
3. 不引入 flags、场景 builder 或第二个 filesystem owner；测试断言与 fixture 内容不变。

## 验证

- 重复模式反向搜索、targeted rustfmt、代表性 package/schema/resource target tests、`git diff --check`；
  不跑全量 compiler gate。

提交 clean；回报 API、迁移 call sites、测试与 blocker。
