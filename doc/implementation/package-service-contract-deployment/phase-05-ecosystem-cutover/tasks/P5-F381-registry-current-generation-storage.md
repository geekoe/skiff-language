# P5-F381 Registry current-generation storage

状态：Ready。

## 直接父节点

- `P5-F375-registry-generation-revalidation-result.md`
- `P5-F377-registry-service-call-authoring-result.md`

F377已经恢复真实20-operation contract。本节点只迁移Registry对当前canonical identity generation的
校验，并补齐四类immutable/pointer动态成功路径。

## Checkpoint与worktree

- skiff-packages integration：
  `3653a294cfb92e60e220dcccc94bc8e8add65b33`
- worktree：
  `/Users/geek/workspace/skiff-packages-p5-f381-registry-current-generation-storage`
- branch：
  `codex/p5-f381-registry-current-generation-storage`
- Skiff toolchain必须使用合入F378 Router启动修复后的phase-05 integration，开始时记录exact commit/tree。

## Production要求

迁移`registry/immutable_store.skiff`对四类摘要的schema/identity generation校验：

| record | canonical generation |
| --- | --- |
| PackageArtifact | schema `v7`、build `v8`、Local ABI `v6` |
| ServiceContract | schema/protocol `v4` |
| ServiceDeployment | schema/artifact `v2` |
| RuntimeAssembly | schema/identity `v2` |

只更新明确的current canonical generation，不放宽为接受任意prefix/version，不保留“旧或新均可”的兼容
分支。Skiff尚未发布，不需要兼容旧generation。

## 动态测试要求

更新`tests/registry/immutable_store.test.skiff`与
`tests/registry/pointer_store.test.skiff`，使用fresh当前generation identity：

1. 四类record分别完成immutable put、相同内容replay、read并验证内容/identity；
2. 四类pointer分别完成：
   - 初始CAS；
   - 第二次CAS；
   - current read；
   - ascending history；
3. 保留并适配现有candidate/release mismatch、非法history limit等负例；
4. 不用伪造旧generation绕过production校验；测试数据应来自fresh实际receipt或严格符合当前canonical
   identity格式。

运行：

```bash
cd /Users/geek/workspace/skiff-packages-p5-f381-registry-current-generation-storage
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration npm run test:registry
```

必须记录source/receipt/runtime各自非零计数。Router若仍因F378已知export以外的问题无法启动，精确返回
`TASK_SCOPE_EXPANDED`，不得改Skiff。

## 写入边界

允许：

- `registry/immutable_store.skiff`
- `registry/pointer_store.skiff`（仅当前generation校验确需时）
- `tests/registry/immutable_store.test.skiff`
- `tests/registry/pointer_store.test.skiff`
- Registry局部测试清单/fixture。

禁止修改`registry/api.yml`、20-operation contract、其它package、Skiff/Internals、artifact schema、
stable/live。不得加入历史兼容。

完成production/tests本地commit，worktree clean，不merge/rebase/push。返回四类成功矩阵、负例、exact
commit/tree和测试计数；主Agent写result。新Agent执行，不派子Agent。
