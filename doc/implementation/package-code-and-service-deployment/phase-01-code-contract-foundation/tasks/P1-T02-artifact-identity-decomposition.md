# P1-T02：拆分 Artifact Identity 单一 Owner

状态：`ready`
类型：营地前置任务，行为等价
依赖：无
执行者：Artifact Identity Agent，一份提交

## 背景

`artifact-identity/src/lib.rs` 已同时承载 package、service、runtime program、package test、
canonical hashing 和大量测试，超过两千行。T03 将新增 package boundary identity；继续向该文件
追加会使 identity owner 更难辨认。

## 目标

按 identity domain 拆成内部模块，保持所有公开 API、hash 输入、错误 taxonomy、CLI 输出和
golden 完全不变，为 T03 留出明确扩展点。

## 范围与 ownership

允许修改：

- `artifact-identity/src/lib.rs`
- `artifact-identity/src/` 下新增模块
- `artifact-identity/tests/` 中仅因模块移动需要调整的测试

建议职责边界：

```text
canonical/          canonical bytes/hash helpers
package.rs          PackageUnit build/ABI identity
service.rs          ServiceUnit identity
runtime_program.rs  runtime graph/build identity
package_test.rs     package-test assembly identity
error.rs            public error taxonomy
```

具体文件名可调整，但不能建立第二套 hash helper 或改变 re-export surface。

## 非目标

- 不增加 `boundaryAbiIdentity` 或新 DTO；由 T03 完成。
- 不改变 schema version、serde shape、hash algorithm 或 canonical field ordering。
- 不清理与 identity 无关的 artifact resolver。
- 不因“更合理”而重新生成现有 golden。

## 实现约束

- `lib.rs` 只保留 public re-export、跨域入口和最小 glue。
- canonical serialization/hashing 只有一个 private owner；domain 模块提供 typed identity input。
- compiler 侧 wrapper 仍必须最终调用本 crate，不能复制 hash 算法。
- 测试可按 package/service/runtime/test domain 移入模块，但不得丢失既有断言。

## 验收标准

- diff 中只有模块移动、可见性/导入调整和等价测试组织。
- 所有既有 identity/golden/CLI 测试原样通过。
- `lib.rs` 不再包含多个 domain 的大段实现或内联测试。
- `scripts/check-artifact-identity-single-source.mjs` 通过。

## 聚焦验证

```bash
cargo test --no-fail-fast -p skiff-artifact-identity
node scripts/check-artifact-identity-single-source.mjs
git diff --check
```

## 停止条件

- 拆分必须改变 canonical bytes 或 public API 才能完成；
- 发现 compiler/projection 中存在真实的第二套 identity 算法而非薄 wrapper；
- 测试只能通过更新 golden。

以上情况先报告，升级为独立修复任务，不能混入机械拆分。

## 提交

提交信息建议：`refactor(artifact-identity): split identity domains`
