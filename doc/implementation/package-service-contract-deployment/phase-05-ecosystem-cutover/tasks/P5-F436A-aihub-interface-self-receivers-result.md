# P5-F436A AIHub interface explicit self receivers result

状态：`IMPLEMENTATION_PASS / CANONICAL_NEXT_BLOCKER`。

AIHub 两个 interface 的五个 method 已全部显式声明 `self: Self`，对应 concrete `impl`
signature、返回类型和业务实现未变。canonical AIHub `type-check` 不再报告 F435A 冻结的
object-safety 诊断，随后在 package publish 阶段停止于独立的
`skiff.run/http-session@1.0.0` `PackageArtifact` pointer 缺失。本文按任务停止规则只记录新首错，
没有承接 package owner、运行后继测试或扩大写集。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| Skiff integration 输入 | `6276ddbea46184ccc4251aa3173ab411f38ac28a` | `8c8db96daba50e7040211cc02549b2a04086ed60` |
| Skiff task dispatch 输入 | `fb06108be8ea9c370216f52891fddddb1ccca340` | `43ca843d6e19e73e37ff7f81c03191c40c1dc29f` |
| Internals integration 输入 | `58950858a2e2cbf2bd95443d5e0704d0d29e7706` | `db88355a103e6e1939e9969756501c7f656c1344` |
| Internals implementation | `2e8ca6110bc2ebd3e07d7dc988eb0fa0318cc412` | `23be114f0d4b838eff1c7b214a40fc9c57cdd354` |

Internals implementation 只修改：

- `aihub/service/internal/aihub_service.skiff`
- `aihub/service/internal/provider_catalog.skiff`

Skiff 只新增本文 result。没有修改 compiler、runtime、router、test-runner、AIHub HTTP
payload/stream、provider transport、service/API authoring、Agine、Codex Relay 或
skiff-packages。

## 2. 实现与反向检查

`AihubManagedLlmClient` 的三个 method：

- `validateChat(self: Self, ...)`
- `streamChat(self: Self, ...)`
- `webSearch(self: Self, ...)`

`AihubProviderCatalog` 的两个 method：

- `builtinProvider(self: Self)`
- `model(self: Self, ...)`

tracked `aihub/service/**/*.skiff` 的反向搜索仍只找到这两个 interface declaration，并在两者
的全部五个 method 上找到首参数 `self: Self`。implementation diff 只改变上述五个 interface
signature；concrete `AihubManagedLlm` 和 `AihubProviderCatalogService` 的五个 impl signature、
返回类型与方法体均为 0 diff。

## 3. 验证

| 命令 / 检查 | 结果 |
| --- | --- |
| `rg -n '^interface\s' aihub/service --glob '*.skiff'` | PASS，只找到两个 canonical interface |
| 五个 `self: Self` signature 反向搜索 | PASS，5/5 |
| `git diff --check` | PASS |
| `SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f436a-aihub-interface-receivers npm --prefix aihub/service run type-check` | `FAIL_NEW_BLOCKER`，exit 1；已越过 F435A object-safety 首错 |
| 最小 AIHub source/interface 聚焦测试 | `NOT_RUN_BY_CONTRACT`；只有 `type-check` 通过才允许运行 |

canonical workflow 使用 linked-worktree 隔离路径和临时 ecosystem store。它不再输出：

```text
interface selector `AihubManagedLlmClient` is not object-safe:
method validateChat must declare `self: Self` as its first parameter;
method streamChat must declare `self: Self` as its first parameter;
method webSearch must declare `self: Self` as its first parameter
```

## 4. Failure classification

新首错为：

```text
error: package dependency skiff.run/http-session@1.0.0 has no published PackageArtifact pointer
```

分类：

```text
NEXT_INDEPENDENT_PACKAGE_DEPENDENCY_POINTER_BLOCKER
  stage: canonical isolated service graph / package publish
  missing dependency: skiff.run/http-session@1.0.0
  missing record: published PackageArtifact pointer
  repaired owner crossed: AIHub interface object safety
  blocked:
    canonical type-check PASS
    optional minimal AIHub source/interface focused test
```

该诊断不要求继续改变 AIHub interface declaration，也不属于本 leaf 允许写入的 owner。需要由
独立 package publication/pointer owner 判断缺失记录的原因；本文没有推断或修改 publish
ordering、package artifact、compiler 语义或 skiff-packages。

## 5. 隔离与禁令

- 未启动、读取或修改 stable instance、watch registry、artifact root、router、runtime、
  telemetry、MongoDB 或固定端口 workload。
- 未运行 `build`、`dev`、`start`、reload、完整 combined、live provider 或 Agine chat smoke。
- 未 merge、rebase 或 push。
- implementation 与 result 分开提交；result commit/tree 和两个最终 clean 状态由交付消息记录。
