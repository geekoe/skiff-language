# P5-F152：Imported HTTP Boundary Types

状态：Ready

## 父节点

- `P5-D86-http-ingress-boundary-availability-audit-result.md`

## 写入与完成标准

- owner：artifact-model canonical HTTP identity与compiler/projection boundary type closure/projection及真实source integration tests。
- official `skiff.run/std` canonical `std.http.HttpRequest/HttpResponse/HttpResponseStreamEvent` PackageSymbol精确投影为既有
  HTTP boundary types；支持真实source/import与`Stream<HttpResponseStreamEvent>`。
- 同名非official package symbol、HttpClientRequest、错误arity、嵌套capability继续fail closed。
- 不改变source PackageSymbol一般语义，不按display name admission，不触碰unknown call/effect eligibility。

先列出并运行compiler projection与real-source聚焦tests；格式、`git diff --check`。不运行完整gate。

worktree `/Users/geek/workspace/skiff-p5-f152`，branch `codex/p5-f152-imported-http-boundary`。
提交、不push、不操作stable。

