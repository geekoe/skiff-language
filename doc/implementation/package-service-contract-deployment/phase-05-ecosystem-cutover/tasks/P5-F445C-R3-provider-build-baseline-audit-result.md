# P5-F445C-R3 Provider build baseline audit result

状态：`AUDIT_PASS`。没有触发 `TASK_SCOPE_EXPANDED`。

本只读 leaf 在不包含 F445D/F445G 的可独立编译 pre-I3 worktree 中，确认 provider package
build、Local ABI 以及 publication receipt 的既有基线。没有修改测试或 production。

## 1. 输入与范围

| 项目 | Commit | Tree |
| --- | --- | --- |
| R2 golden 闭合结果 | `1b6b0d86` | `a2a67586b908b21472e6e656c76ea6906a0dd8ed` |
| R3 task dispatch | `72fbe117` | `9bca08a2dd0e14d1dc46ebbddeb94ce497cb0d92` |

审计对象：

```text
compiler/tests/package_interface_identity.rs
direct_package_interface_identity_normalizes_dependency_and_package_id_owners
```

## 2. Pre-I3 identity 基线

聚焦测试完整通过，因此测试内依次执行的断言共同证明：

| 观测项 | Pre-I3 值 / 结论 |
| --- | --- |
| provider package build | `skiff-package-build-v10:sha256:3b9f3647318e5da0a7698be305309f5b18f0e0cbfdf256b6fc1fd7d5162116ef` |
| provider Local ABI | `skiff-package-local-abi-v7:sha256:2b6b70c8b858a3ee88df957eb0488a98224fd928669c84021f15aecf7de464e6` |
| publication receipt | 等于 provider package build |
| consumer 内 provider artifact | 与 standalone provider artifact 字节相同 |
| path-free provider receipt | consumer 与 standalone 相同 |

这给 F445G 提供了明确的 pre-I3 对照：File IR 持久格式升级前，provider build 是
`3b9f3647…116ef`，Local ABI 是 `2b6b70c8…64e6`，receipt 仍直接使用 provider build identity。

## 3. 命令与结果

使用独立 target `build/cargo-target-r3-provider` 运行：

```text
cargo test -p skiff-compiler --test package_interface_identity \
  direct_package_interface_identity_normalizes_dependency_and_package_id_owners -- --nocapture
```

结果：

```text
1 passed; 0 failed; 3 filtered out
```

Cargo 只报告仓库既有 unused/dead-code warnings。

## 4. Scope 与禁令

- 除本文 result 外没有修改任何文件。
- 没有修改测试常量、production、identity 算法或 F445G 实现。
- 没有派生子 Agent。
- 没有启动 stable instance 或执行 live workload。
- 没有访问 network。
- 没有 merge、rebase 或 push。
