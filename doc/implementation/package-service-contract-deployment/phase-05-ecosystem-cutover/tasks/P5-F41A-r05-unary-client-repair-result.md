# P5-F41A：R05 Unary Client Repair Result

结论：COMPLETE，implementation checkpoint。

- task commit：`70548a73b0596042e1cbdc0c9b975166b57cd24f`
- integration commit：`8c832b44a49b31da393064ab2c6c7d432db70274`
- integration tree：`9f55ccc9afc87b4d3d350e3dd416f5150149e343`
- Cargo.lock blob：`f3ce5457138c58aec4c84abda431afa96013e3fd`

修复只触及lifecycle real unary client与direct test。client现在使用`node:http.request`向isolated动态Router URL
显式发送receipt-owned `POST /probe`及wire Host，无retry、fallback、stable端口或额外selector。direct本地HTTP
server观察真实outbound wire并验证B marker；404 diagnostic包含method/URL/wire Host/status及脱敏、512-byte上限
body、原始bytes/truncated。

开发owner证据：合同`node --check` PASS，direct 7/7 PASS，worktree clean。未运行真实transcript、combined、
instance、stable或完整gate。I31 author/store证据未失效；R05失败证据仍需新周期复验。
