# Skiff 文档

本目录保存 Skiff 的 canonical 文档集合。当前概览入口是 `overview.md`。

## 目录结构

- `overview.md`：语言定位、工程模型和关键设计取舍。
- `reference/`：稳定规则，包括 syntax、static semantics、interface、runtime、DB、queue、spawn、observability、file and command、publication、std surface 和 testing。
- `architecture/`：长期内部架构契约，包括 compiler/runtime/router 边界、DB capability、release/registry、artifact linking、未规范化问题和共享协议 fixture；不作为用户语言规范。

## 维护规则

- 文件名和当前规范阶段不再使用 `v1` 命名；`skiff-*-v1` 这类字符串只作为当前 wire/schema/identity 字面量保留，不能反推出文档阶段名。
- 事实只写在负责该主题的文档里；`overview.md` 只保留定位、工程模型和关键设计取舍。
- 设计理由放在具体规则旁边，避免单独维护中央规则表。
- 目录结构承担导航；不要在正文里维护长阅读索引或交叉索引网。
- 不要在文档里复制大段代码、配置或临时实现计划；这些内容最容易随实现漂移。
- 临时实现计划、执行记录和历史草案不属于公开 canonical 文档；仍未规范化的问题才放入 `architecture/open-issues.md`。
