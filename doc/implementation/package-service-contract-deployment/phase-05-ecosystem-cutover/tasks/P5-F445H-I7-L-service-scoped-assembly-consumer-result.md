# P5-F445H-I7-L Service-scoped assembly consumer result

状态：

```text
PASS
L_COMPLETE = YES
HOST_CONSUMER_COMPLETE = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```

L已经把assembly resolver、loader、linker、Host activation/wire admission、runtime request执行和
package-test入口统一到：

```text
ServiceIngressKey = (ServiceDeploymentRef, IngressSelector)
IngressSelector = (protocol, method, path)
```

不同service deployment可以拥有相同入口；同一精确deployment的重复入口、同service/version多
revision、错误deployment、旧代际和错assembly/gateway identity均失败关闭。HTTP URL中的Host只保留
为请求元数据，不参与service route选择。

## 1. Parents and checkpoints

| 项 | 值 |
| --- | --- |
| design parent | `P5-F445H-I7-D0-service-scoped-ingress-design-result.md` |
| canonical parent | `P5-F445H-I7-K-service-scoped-ingress-canonical-result.md` |
| initial baseline commit/tree | `1a11328a241b5d177eb40885e294fe31d65a7240` / `ca1f7c2f040458df4275d00801eb0fc61046a1a8` |
| independent implementation commit/tree | `79b7cb04c4b223bae96a8b4dd5baca293ff7c576` / `6dcc6340db43e84251260444d73547daef86cb7b` |
| continuation implementation commit/tree | `8ff6b953` / `7f6aa6f90b1b1e2e58ffd5bd5bb1f3ce973e7c56` |
| final dependency baseline commit/tree | `23495b684992d6f552c8e849ae88c6e9d5d89c5f` / `ee6bcf919ef3e40d727f57aebcc141ce192d408d` |
| final fixture closure commit/tree | `d06695c5` / `bd16056153813eef7fd76363c0823fd87d6d0dc5` |
| branch | `codex/p5-f445h-i7-l2-host-wire` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-l2-host-wire` |
| integration owner | `/root/phase05_integration_steward` |

最终result commit/tree在Git handoff中报告；result不能自引用自己的commit identity。

## 2. Implementation

- assembly resolver以`ServiceIngressKey`索引入口，并允许不同service共享相同
  `protocol + method + path`；
- loader、linker和Host active route保留精确deployment，查找不再退化为裸selector；
- Host从runtime frame携带的deployment构造精确key，并校验HTTP、WebSocket connect和WebSocket
  JSON-RPC method route的deployment一致；
- WebSocket generation继续固定旧连接所属的immutable route，但方法选择不再使用Host字段；
- runtime request的HTTP与WebSocket connect执行事实显式包含精确deployment，跨deployment替换失败；
- package-test入口也使用`ServiceIngressKey`，不能由另一个deployment的同形入口替代；
- Host旧RuntimeAssembly v2测试fixture升级到v3，旧DeploymentArtifact v3 golden升级到v4；
- URL Host不同但deployment、method与path精确一致的HTTP请求有正向执行证据。

没有增加legacy兼容、dual-read、fallback或ambient Host推导。

## 3. RED and GREEN evidence

### RED

独立checkpoint先以旧fixture运行`cargo test -p skiff-deployment`，真实失败是
`IngressSelector.host`仍存在。continuation dependency join依次暴露并关闭：

1. compiler仍构造旧Host字段；
2. test-runner仍构造旧Host字段；
3. package-test仍以裸selector查询；
4. Host test fixture仍使用RuntimeAssembly v2以及把URL Host误当route条件。

每个外部owner断点均等待对应C/F节点进入integration，没有越界修改。

### Final GREEN

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-deployment --no-fail-fast` | PASS，61/61 |
| `cargo test -p skiff-runtime-loader --no-fail-fast` | PASS，19/19 |
| `cargo test -p skiff-runtime-linker --no-fail-fast` | PASS，58/58 |
| `cargo test -p skiff-runtime-request --no-fail-fast` | PASS，41 unit + 1 doc |
| `cargo test -p skiff-runtime-package-test --no-fail-fast` | PASS，8/8 |
| `cargo test -p skiff-runtime-host --no-fail-fast` | PASS，329 unit + 2 + 6 + 2 integration + 1 doc = 340 |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

既有unused/dead-code/unreachable-pattern warnings不属于本节点引入的行为失败。

## 4. Exact behavior evidence

- 不同service deployment共享同一`GET /v1/models`：resolver正向通过；
- 同一deployment重复selector：resolver失败；
- 同service/version多revision：assembly resolution失败关闭；
- loader/linker/Host activation：精确deployment随入口保持；
- HTTP与WebSocket connect错误deployment：request执行校验失败；
- WebSocket JSON-RPC错误deployment：Host wire admission失败；
- 相同path但错误deployment：Host route lookup无候选；
- 不同URL Host、精确deployment/method/path：Host正向执行成功；
- stale assembly generation、错误assembly identity和gateway identity：均失败关闭。

## 5. Reverse search and write boundary

在`runtime/host`、`runtime/request`和`runtime/package-test`中反搜：

- `selector.host`、`ingress.host`：0；
- `skiff-runtime-assembly-v2`：0；
- 已知裸selector ingress lookup：0。

continuation相对最终dependency baseline只修改`runtime/host/**`、`runtime/request/**`、
`runtime/package-test/**`及本任务/result文档。compiler、Router TypeScript、artifact-model和
artifact-identity canonical owner均未越界修改。

没有push，没有访问stable/live/network/Mongo/OAuth/browser。

```text
L_COMPLETE = YES
HOST_CONSUMER_COMPLETE = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```
