# P5-F445G-R1 Router File IR v9 admission result

状态：`IMPLEMENTED / ROUTER_FULL_BLOCKED_BY_INHERITED_ASSERTION`。

## 1. 输入、写集与提交

| 项 | commit |
| --- | --- |
| F445G implementation base | `dee2d0b5d67df9a6f3358d68ee835c7695680e21` |
| task | `20fe98bc` |
| implementation | `a4b1926d` |

implementation 严格只修改两个指定 Router consumer：

- `filesystemRuntimeAssemblySnapshotLoader.ts` 只接受 exact
  `skiff-file-ir-v9:sha256:`；
- `compilerGeneratedManifestCompatibility.test.ts` 只断言 compiler 当前生成的 exact v9 identity。

没有增加 v8 fallback、dual-read 或兼容分支，也没有修改 Rust、compiler fixture generator、
artifact schema、其它 identity 或 Router 行为。全 Router 反搜确认没有第二个生产 v8 consumer；
`artifacts.test.ts` 的 v3 字面量属于旧式 ServiceUnit 不完整指针拒绝用例，不是当前
compiler-generated File IR consumer。

## 2. Test-first 证据

修改前执行任务指定命令：

```text
pnpm --dir router test -- tests/compilerGeneratedManifestCompatibility.test.ts
```

该 package script 会把额外的 `--` 传给 Vitest，实际展开为 Router 全套测试。目标测试如期 RED：

```text
expected /^skiff-file-ir-v8:sha256:...$/
received skiff-file-ir-v9:sha256:9a86474b...
```

同次运行的 `dynamic-build-id-parity.test.ts` 也因 loader 的 v8 prefix 拒绝同一 v9 记录，
报 `fileIrIdentity is invalid`。这证明 reader 与 expectation 是本任务的两个失败来源。

## 3. 修改后验证

| 命令 | 结果 |
| --- | --- |
| `pnpm --dir router exec vitest run tests/compilerGeneratedManifestCompatibility.test.ts` | PASS：1/1 |
| `pnpm --dir router exec vitest run tests/dynamic-build-id-parity.test.ts` | PASS：4/4 |
| `pnpm --dir router type-check` | PASS |
| `rg -n 'skiff-file-ir-v8' <两个指定文件>` | PASS：零匹配 |
| `git diff --check` | PASS |
| `pnpm --dir router test` | 57/58 files、819/820 tests PASS；见下述既有阻塞 |

Router full 的唯一失败是修改前已经存在的
`actor-spawn-runtime-control.test.ts` 错误文本断言：生产 validator 已列出
`connection.request` 和 `connection.request.cancel`，测试期望文本尚未列出。该失败在本任务
修改前的 RED 运行中已经出现，与 File IR v9 无关，且不在本任务两个文件的写集内，因此未越权修改。

## 4. 边界

- current compiler-generated fixture 已确认是 v9。
- 只有一个 production File IR identity admission consumer。
- File IR 相关 direct compatibility 与真实 filesystem loader 覆盖均为 GREEN。
- 没有派子 Agent，没有 merge、rebase、push、stable、live 或 network 操作。

