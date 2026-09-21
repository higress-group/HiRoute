# HiRoute 安装候选组件声明

[English](DISTRIBUTION-NOTICE.md)

HiRoute Rust workspace 声明 Apache-2.0。安装包包含 HiRoute Desktop、hirouted、hiroute、
前端资源、内置模型目录及固定版本 CLIProxyAPI (CPA)。各组件保持各自版本和许可。
CPA 的版本、源码 commit、最终签名字节摘要见 installation.json。

第三方 Rust/npm 依赖的版本和声明许可见 dependency-inventory.json。
`Licenses/THIRD-PARTY-LICENSES.txt` 及其 JSON 清单由打包器从锁定依赖与固定 CPA commit
确定性生成，包含各组件随源发布的 LICENSE、NOTICE 等材料；CPA 与 HiRoute 自身许可证也位于
同一目录。工具不会用 HiRoute 的许可替代第三方许可，发现缺失材料时会拒绝打包。

`controlled-trial` 只表示包采用自签名且未公证，不表示省略许可证材料。
