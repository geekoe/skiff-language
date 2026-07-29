# P5-F445H I7 P8 X Independent HTTP entry acceptance

状态：

```text
BLOCKED_BY = I
READ_ONLY = YES
```

## 1. Inputs

- 直接父节点：
  `P5-F445H-I7-P8-I-aihub-http-entry-migration.md`
- ancestry floors：
  - Skiff `3a87d37f81a04c249f308b311bd91dcfdf3a8aa3` /
    `eafc29e952f6b5170e4f5faca4e5d181b3ace9f6`
  - Internals `9c3bdc82c4a43e575ea627357c05f54dbc0400a8` /
    `c3f159a397cd3c2b316a502ce945d8a935a9c2c3`
- verdict前必须冻结并报告K/H/R/T、S1 diagnostic、S2/S3与I合流后的精确Skiff与Internals
  commit/tree；任一变化使verdict失效。

## 2. Acceptance matrix

独立检查：

1. 没有新增标准库/语言/File IR/wire/schema/test session/header机制；
2. test service显式`http.yml`，不自动投影subject ingress；
3. 普通绝对URL经isolated Router business port、现有service/version和method/path到达wrapper；
4. entry内部effect消费父registry，子不finalize、父唯一finalize；
5. Router production保持普通路由；Host不路由；
6. forbidden header覆盖、错误origin、缺失ingress URL、并发子请求、cross-case identity均fail closed；
7. stream break/drop复用普通cancel/backpressure；
8. HTTP child与parent stream registry隔离，只在child当前runtime从effect wire snapshots生成handle；
9. S1保留普通wrapper→`PackageDirect` return stream的真实GREEN证据，但状态仍为
   `TASK_NOT_EXECUTABLE`，不伪造production closure；
10. 同一HTTP child request内，overlay-local producer作为dependency `PackageDirect` producer参数时，
    三个stream的registry/generation、create/lookup、cancel/finish与executable轨迹闭合；heap间item只由
    既有`StreamInternalItem`/wire搬运；
11. deferred dependency producer只继承raw HTTP gateway既有response sink；HTTP context外仍fail
    closed，service stream仍经过boundary materialization；
12. AIHub四条按完整body/SSE event断言并GREEN；
13. 反事实审查：删除任一P8新增机制后若仍满足上述标准，该机制必须判为多余并阻塞。

只运行最小聚焦抽查，不重复T/I仍有效的昂贵证据。输出`PASS/FAIL`、blocking issues、
non-blocking follow-up、精确命令和残余风险；不得修改文件或顺手修复。
