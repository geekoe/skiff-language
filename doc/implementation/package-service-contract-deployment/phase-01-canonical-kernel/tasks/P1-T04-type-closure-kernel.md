# P1-T04：Nominal Type Closure Kernel

## 目标

把boundary、recoverable和spawn路径重复的TypeRef递归、nominal declaration解析、guard状态与trace生成
收敛到compiler-core，使后续Package boundary analysis不需要复制projection实现。

## 依赖与 worktree

- 无前置代码任务；可与T01/T03并行。
- 建议branch：`codex/package-service-p1-t04-type-closure-kernel`。

## 完成态

1. `compiler-core`在现有`type_ref`/`type_graph`基础上提供projection-neutral resolver trait、nominal
   declaration source、closure walker、cycle guard和typed trace/path结果。
2. kernel覆盖native generic、record、union、nullable、function param/return、any-interface args、local/
   publication/package/service nominal refs；policy通过callback/trait注入，不知道boundary或recoverable业务。
3. `contract/boundary.rs`与`recoverable_boundary.rs`共享kernel和同一种package type source，不再各自递归
   declaration closure或复制display path算法。
4. spawn/recoverable validation对同一type closure只执行一次；artifact projection不重复调用完整schema
   validation。
5. 直接触碰的超长文件按resolver、policy、diagnostic adapter拆分；不得把旧代码整体搬进另一个巨型文件。
6. 所有现有accept/reject语义和稳定diagnostic reason保持；只允许行文细节变化，测试按typed reason/trace
   断言。

## 写入范围

- `compiler/core/src/type_ref.rs`、`type_graph.rs`及新增closure modules/tests。
- `compiler/projection/src/contract/boundary/**`、`recoverable_boundary/**`、
  `service/spawn_targets.rs`、`service/artifact_projection.rs`的直接迁移。

不要修改artifact DTO、effect facts、identity、PackageUnit builder或runtime。

## 验证

```bash
cargo fmt --all -- --check
cargo test -p skiff-compiler-core
cargo test -p skiff-compiler-projection contract::boundary
cargo test -p skiff-compiler-projection recoverable
cargo test -p skiff-compiler-projection spawn
node scripts/check-compiler-boundaries.mjs
git diff --check
```

若cargo test filter不匹配实际test名，Agent必须记录替代的精确crate/test命令。增加共享kernel测试覆盖完整
path、guarded recursion、cycle、missing declaration和package dependency trace。

## 自验收与回报

反向搜索`BoundaryPackageTypeSource`、`RecoverablePackageTypeSource`、自定义walker和重复schema validation；
提供删除/保留解释。提交自验收矩阵、文件行数变化和commit。
