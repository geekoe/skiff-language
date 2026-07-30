# P5-R21：Gate Causal Evidence Acceptance Result

`R21 PASS`

全新独立只读reviewer验收`dbfb98ac0a10d3959d803a8a92de1c04bba66fce` / tree
`68a824aa233ade4cd455c7be999f5fa1219b46cc` / lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`上的F21A/B表面，blocking issues为0。reviewer确认v6 causal rank、三条有界诊断、
omitted count、validator fail-closed、脱敏边界与pre-runtime startup marker成立；未把确定性stderr优先误写为跨pipe真实时序。

窄验收重跑与batch combined相同的三个Node test文件，约3.1秒完成，44 pass、0 fail、0 cancelled、0 skipped；
`git diff --check` PASS。未修改候选，未运行I16动态probe、Host/full/stable。

现存v5 combined ledger SHA-256
`244c921ab4efea2bbd3bf20e4f480f7d12af5d535a3b31ab87d722d727a37519`只属于历史候选证据；v6 validator对它
fail closed，不能作为新combined或下一次full的解锁条件。F21C仍是下一独立pending DAG节点，必须先建立任务合同；
R21只验收F21A/B，不是阶段verdict，也不表示阶段完成。
