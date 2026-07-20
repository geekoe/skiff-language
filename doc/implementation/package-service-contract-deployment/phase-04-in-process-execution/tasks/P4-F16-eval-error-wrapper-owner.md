# P4-F16：Eval Error Wrapper Owner Migration

## Blocker、输入与边界

T10在exact clean candidate `f093e921a6c7961c5d727deeb83a2b6fd78adb94`补跑fail-fast未到达项时，
`check-runtime-eval-error-boundary`在`assembly_execution/boundary_materialization.rs`报告4个DENY：本地
`replace_user_exception`直接解构/重建`RuntimeError::WithSource`与`WithDiagnosticFrame`。独立分类确认这是机械
error-owner迁移，不是typed error语义缺口。

权威输入为架构§6–§8、§12、§14，P4-F06、F08、F10、T10合同与eval error-boundary checker。只把wrapper递归
下沉到唯一`runtime/eval/src/error.rs` owner；不得修改checker/allowlist、错误分类、materialization plan或其它lane。

- 依赖：T10@`f093e921` eval error-boundary FAIL与独立分类PASS。
- 解锁：新T10 stability epoch。
- branch：`codex/p4-f16-eval-error-owner`。
- worktree：`/Users/geek/workspace/skiff-p4-f16-eval-error-owner`。
- integration边界：只提交task branch，不merge integration/main、不push。

## 写入范围与完成态

独占`runtime/eval/src/error.rs`及其单元测试、
`runtime/eval/src/assembly_execution/boundary_materialization.rs`与对应tests。不得修改checker、host/native/router或
其它runtime模块。

1. `error.rs`提供唯一窄helper（例如`replace_user_exception_preserving_diagnostics`）：递归到
   `UserException` leaf后替换，其它leaf原样返回；只有该boundary解构wrapper。
2. 每层`source_id`、source frame、diagnostic frame与嵌套顺序完全保持。不得用已有attach helpers重建，因为其
   去重/合并语义不同。
3. planner删除本地递归，只调用canonical helper。`actual_payload_type`、exception envelope元数据与catch identity不变，
   仅把业务payload换成caller heap中的detached value。
4. runtime/cancel/provider错误原类透传；undeclared typed throw、decode/encode/schema/value-plan失败继续是带operation
   target的`Protocol`，不得降级或丢target。
5. 测试覆盖`Diagnostic -> Source -> UserException`嵌套替换、非UserException leaf不变，以及shared planner同时携带
   source+diagnostic frame时的detached payload/catch projection。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-eval service_error_boundary
cargo test -p skiff-runtime-eval replace_user_exception
node scripts/check-runtime-eval-error-boundary.mjs
git diff --check
```

每个Rust filter必须非空。反向搜索`replace_user_exception`与production wrapper constructor/method，完成态只能由
`error.rs`访问wrapper内部，test-only构造不计；F08/F10范围不得出现第二helper。

## 回报

提交一个clean commit，回报wrapper结构保持、typed/catch语义、反向搜索、checker修复前后、命令、自验收矩阵和
extra-review。`error.rs`虽长但仍是架构唯一boundary；不得为本任务创建第二error owner。
