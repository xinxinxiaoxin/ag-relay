# 第三方组件与许可声明

## 授权边界

AG Relay 自有代码采用根目录 `LICENSE` 中的 MIT 许可证。依据 ADR-0007，维护者确认原网关核心及导入设计代码由其借助 AI 开发，并授权以 MIT 分发；网关核心纳入本项目自主维护，不再以整批覆盖方式跟随原仓库。此授权不改变第三方代码的权属，不改变 `relay-retry-proxy` 仓库可见性，也不表示已执行公开或发布操作。

`src/components/ui/`、`src/hooks/use-mobile.ts` 等保留或改编自 shadcn/ui 的组件不能仅以项目自有 MIT 声明替代原声明。其原始 MIT 版权为 **Copyright (c) 2023 shadcn**；相应完整原文已纳入本材料。其他组件、图标与字体仍各自遵守下列条款。许可英文原文保持原样，不用中文摘要替代。

## 材料入口与随包布局

源码目录的 `licenses/` 包含完整材料；安装后它整体位于安装目录的 `licenses/third-party/`。以下路径相对于这个材料根目录，在源码和安装包中保持一致：

| 路径 | 内容 |
| --- | --- |
| `COMPONENTS.md` | 人类可读的逐组件版本、许可、范围、原文链接及下载来源 |
| `inventory.json` | 核验清单：每项的实际依赖元数据、来源、锁定版本、原文路径及 SHA-256；含排除的平台包 |
| `texts/*.txt` | npm / Rust 依赖的完整 LICENSE、NOTICE、COPYRIGHT、AUTHORS 等原文；按内容 SHA-256 去重，未删减或改写版权 |
| `node/LICENSE` | Node.js **v24.18.1** 的完整复合 LICENSE，包含其内含第三方的全部条款，不是只有顶部的 MIT 段落 |
| `node/SHASUMS256.txt` | 该 Node 版本的官方二进制摘要清单 |
| `rust-std/` | Rust **1.99.0** 标准库的 MIT / Apache 原文及官方 `COPYRIGHT-library.html`（含内置第三方） |
| `webview2/` | Microsoft.Web.WebView2 SDK **1.0.3800.47** 原始 `LICENSE.txt` / `NOTICE.txt` |
| `sources/*.crate` | MPL-2.0 Rust 组件未修改的完整对应源码包（`.crate` 为 tar.gz），摘要与 Cargo.lock 一致 |
| `installer/nsis/` | NSIS 3.11 的完整 COPYING、NSISdl / Modern UI 2 原文，以及固定版本完整对应源码；Modern UI 2 另附仅转码的 UTF-8 可读副本 |
| `installer/wix/` | WiX 3.14.1.8722 的 MS-RL 原文与 `wix3141rtm` 固定提交的完整对应源码 |
| `installer/nsis/tauri-utils/` | 官方插件 0.5.3 的根许可、固定源码及 Chromium 移植版权；`reference-licenses/` 只是明确标注的声明依赖许可参照，不能证明旧 DLL 实际编译版本 |

项目 MIT 和本声明分别映射为安装目录的 `licenses/LICENSE` 与 `licenses/THIRD_PARTY_NOTICES.md`。分发安装包或裸目录时须一并保留这两个文件和整个 `licenses/third-party/`，包括源码包，不能只附链接、仅附根 MIT 或只带主程序。

## 主要组件

下面只是导航，不替代 `COMPONENTS.md` 中的完整直接/间接依赖清单，也不将不同版本合并为同一授权。

| 组件 | 实际许可与保留事项 |
| --- | --- |
| React、React DOM、Radix UI、shadcn/ui、Tailwind CSS | 实际包元数据与许可原文为 MIT；保留各自的版权和授权文字。shadcn 的 CLI 在开发依赖中，但导入的组件和 `shadcn/tailwind.css` 会进入产品，故仍附完整声明。 |
| Lucide React | 元数据为 ISC，原始 LICENSE **还包含 Feather 来源图标的 MIT 声明**（Copyright (c) 2013-present Cole Bemis）；两部分全部保留，不能只写 ISC。 |
| Geist / Geist Mono | Fontsource 变量字体包元数据为 OFL-1.1，附 The Geist Project Authors 的原版权及完整 SIL OFL 1.1。字体不因随产品分发变成 MIT；禁止单独出售字体，修改字体须遵守 OFL 的名称及再分发条件。 |
| Node.js | Node 自身 MIT，内含的 V8、OpenSSL、libuv、ICU、zlib 等按 `node/LICENSE` 的逐项条款分发。`src-tauri/binaries/node-x86_64-pc-windows-msvc.exe` 的 SHA-256 为 `ac51903c4c111815d52280b1fdcc8da067cbb37e2fe1a765097b85c3292c8582`，与官方 v24.18.1 `win-x64/node.exe` 摘要一致。 |
| Tauri、updater 与其他 Rust crates | 逐包核实 Cargo 元数据及原文，包含 MIT / Apache-2.0 的双许可，以及 BSD、ISC、Unicode-3.0、MPL-2.0 等；不能概括为全部 MIT。`AND` 条款同时适用；`OR` 是相应组件提供的许可选择，原文全部保留。 |
| Rust 标准库 | 实际构建工具链快照为 1.99.0；保留官方标准库专用复合版权文件，包括 compiler-builtins 等内部组件。更换构建工具链后须重新核验这部分材料。 |
| WebView2 Rust wrapper / SDK loader | wrapper 的 MIT **不覆盖**微软的 loader。NuGet 1.0.3800.47 的 `.nuspec` 明确指向 `LICENSE.txt`，原文为微软 BSD-3-Clause 条款，另附完整 NOTICE；其 x64 `WebView2LoaderStatic.lib` 与锁定 crate 携带文件逐字节匹配。这里不是把整个 WebView2 Runtime 宣称为 BSD 或 MIT。 |
| 其他 npm 工具和数据 | 完整树还包括 ISC、Apache-2.0、BSD、0BSD、BlueOak、CC-BY-4.0（caniuse-lite）、MPL-2.0（Lightning CSS）等，原文及归属见清单。仅作为构建工具使用不代表把其整个包分发到运行时。 |

## MPL 对应源码与修改状态

Windows 普通 Rust 依赖树包含 `cssparser`、`dtoa-short`、`option-ext`、`selectors` 等 MPL-2.0 组件，构建/宏依赖另含 `cssparser-macros`。为接收方提供对应 Source Code Form，材料目录直接附带这些锁定版本的完整、未修改 crate 源码（含原始文件级声明），也在清单中提供精确版本的 crates.io 下载 URL。获取这些源码不收额外费用；不能在再分发时删去这些材料或限制接收方依 MPL 获得、修改及再分发相应源码的权利。AG Relay 未修改这些第三方 crates；MPL 义务适用于相关文件，不意味着整个 AG Relay 改用 MPL。

Lightning CSS 的 npm 原生包用于 Tailwind/Vite 构建，本项目只分发生成的前端资源，不把 `node_modules` 或 Lightning CSS 构建器整体装入产品。若另行分发构建环境/这些二进制，也须提供它们对应的 MPL 源码，不能把本 Windows 产品材料误作任意开发环境再分发的合规声明。

## 安装器运行部分与对应源码

实际构建工具为 NSIS **3.11**（`makensis v3.11`）与 WiX **3.14.1.8722**（`candle 3.14.1.8722`）。版本以只读版本查询及实际生成的安装输入核实，并记录工具摘要；没有运行生成的安装器。安装模板本身带有本机构建路径，不复制进许可材料或公开资产。

- **NSIS**：COPYING 列明其主体及通常插件使用 zlib/libpng，bzip2 模块使用自身条款，LZMA 模块使用 **CPL-1.0 并附 NSIS 专用 linking 例外**。实际生成脚本使用 LZMA 压缩、Modern UI 2 及 NSISdl，故全部保留原文和原版权。`installer/nsis/nsis-3.11-source.tar.gz` 为固定版本完整、未修改源码，包含 LZMA 模块及原始文件级许可；源码可直接从随包材料取得，无需付额外费用。linking 例外不将 AG Relay 自有代码改为 CPL，但不免除对应模块的许可义务。对该模块及其所有贡献者的保证、条件与赔偿责任排除依原 CPL 条款执行；任何超出原条款的承诺仅由作出承诺的分发者负责，不代表其他贡献者。
- **WiX**：保留 .NET Foundation and contributors 的版权和 **Microsoft Reciprocal License (MS-RL)** 全文。`installer/wix/wix-3.14.1-source.tar.gz` 为 `wix3141rtm` 对应固定提交的完整、未修改源码，供接收方取得 MSI 中 WiX 标准 UI / 支持代码的对应源码。含 WiX 代码的部分仍受 MS-RL 约束；AG Relay 根 MIT 不能替换其许可或限制接收方的源码权利。
- **实际内嵌闭包**：对 TEST-ONLY 候选包静态提取后确认，NSIS 内有 `System.dll`、`nsDialogs.dll`、`StartMenu.dll`、`LangDLL.dll`、`NSISdl.dll`、`nsis_tauri_utils.dll`；五个标准插件均匹配 3.11 工具缓存，源码与完整 COPYING 已包含，NSISdl 所含 JNetLib 的 Nullsoft 原版权另行保留。MSI 的 `WixUIWixca` 是 `uica.dll` 3.14.1.8722，对应的 `src/ext/UIExtension/ca` 及支持源码包含在完整 WiX 源档中。提取摘要记录在清单的 `embeddedArtifacts`；这不替代最终重构建包的再次比对。
- **Tauri NSIS 插件**：官方 `nsis_tauri_utils v0.5.3` DLL（34,304 字节）与下载常量、官方资产、工具缓存及候选包提取字节一致，SHA-256 为 `5ba143b5db4a87d32d6e7802e033330aae56cbceabe0d1e3ba41948385ad4709`。固定源码为 `13d9edd27b69310e108d6fbd49f90992f8a05390`，自有代码采用 MIT OR Apache-2.0。`run_as_user` 明示移植 Chromium 的 `elevation_util.cc`，因此另附固定来源的 **BSD-3-Clause** 原文及 **Copyright 2024 The Chromium Authors**，没有用插件根许可替换它。
- **预编译插件来源限制（尚未闭合）**：该官方插件不提交 Cargo.lock、不固定 Rust 工具链，历史构建日志已不可取得，DLL 也无可用于还原版本的记录。只能核实源码声明的 semver / windows-sys 等兼容范围，不能把它们当成实际解析的 patch 版本。`no_std` 仍会使用 alloc/core 及可能的 Unicode / compiler-builtins 内容；附带的固定版本许可参照**不是旧 DLL 编译器或静态依赖版本的证明**。这是来源和可重复性限制，不是已发现凭据泄漏或恶意代码。`inventory.json` 的 `releaseBlockers` 保留此未决项；正式发布前须由维护者明确决定并记录接受官方预编译来源及这些限制的审核/例外，或另行取得可信构建记录、受控重建并重新核验。当前正式检查仍拒绝放行，不能只删除记录或填入一个猜测版本。
- **系统前置组件**：本次安装模式为 `downloadBootstrapper`，未将 WebView2 Bootstrapper 或离线 Runtime 嵌入安装包。安装时从微软下载的组件仍适用微软提供的条款，不能依据本材料将其称作 MIT 软件。

## 核验方法、范围与更新

本材料来自 **2026-10-12** 的锁定依赖快照，不是根据常见许可名称猜测：

1. 使用 `npm ci --ignore-scripts` 安装 `package-lock.json` 的 Windows x64 可用包，对照实际 `package.json` 的版本/许可并复制包内许可原文（包括嵌套 NOTICE）。源码中导入的 shadcn 组件和 CSS 也计算在内。其他平台未安装的可选构建工具二进制单独列于 `excludedNpmPlatformPackages`，不是静默遗漏。
2. 使用 `cargo metadata --locked --filter-platform x86_64-pc-windows-msvc --format-version 1 --manifest-path src-tauri/Cargo.toml` 的已解析节点，读取实际 Cargo 元数据和 crate 原文。普通依赖与构建/过程宏依赖标注范围；保守包含完整解析树，不声称这是经链接器裁剪后的精确二进制 SBOM。
3. 发行包没有附 LICENSE 的例外按上游固定提交补原文，清单逐项记录原因和来源。特别是 `react-remove-scroll-bar@2.3.8` 的 npm 元数据为 MIT，但包内没有许可证；保留其上游后来补入的原始 LICENSE（固定提交和版权年份均保留），不伪称发行包自带该文件。平台工具绑定则采用同版本同仓库主包附带的许可。
4. Node 使用对应版本标签的完整 LICENSE，二进制比对 nodejs.org 官方 SHASUMS；WebView2 SDK 来源为精确版本的官方 NuGet 包并核对 loader；Rust 标准库来自实际工具链附带的标准库版权声明。
5. 清单记录了 `package.json`、`package-lock.json`、`Cargo.toml`、`Cargo.lock` 和随包 Node 的指纹。运行 `node scripts/check-licenses.mjs` 进行**离线、只读**校验；缺失材料、许可原文被删节或依赖/二进制变化均非零退出。文本输入允许 CRLF/LF 差异，许可原文自身按原始字节核对。发布校验还须从实际 `rustc --version` 提取版本号，以 `node scripts/check-licenses.mjs --rust-version <实测版本号>` 传入并比对标准库材料；不能硬编码一个期望值代替实测。省略参数只做文件校验，不表示已核验构建工具链。

6. 正式安装包构建必须同时传入 `--rust-version <实测版本号> --installer-runtime`。后者要求清单含 `installer-runtime` 类别的 `NSIS installer runtime` 和 `WiX installer runtime` 两项及其非空材料，并拒绝尚有 `releaseBlockers` 的快照，随后继续校验所有原文指纹。安装器原文固定存放于 `installer/nsis/` 与 `installer/wix/`。首轮仅用于探测实际工具版本、明确标记为 TEST-ONLY 的构建可不要求尚未取得的安装器材料，但不能将这种探测产物当成已通过正式分发许可验收。

升级依赖、改变目标平台、替换 Node、更新 Rust 工具链或新增发行包中嵌入的工具/二进制后，应重新执行上述核验、补齐原文和适用源码、更新 `inventory.json` / `COMPONENTS.md` 并重跑检查，不能只改指纹让旧清单通过。完整性校验不是版权权属审核或法律结论，也不能替代对安装包实际资源的检查。

本材料不包含用户系统预装的 Windows / WebView2 Runtime / 系统字体本体，也不授予它们新的许可。若发行方式增加离线 WebView2 Runtime、安装器附加二进制或其他资源，应按实际版本补相应许可后再发布。正式公开/Release 及历史资产的核查由独立发布流程完成；本工单没有运行现用网关或安装器，也没有发布新版。
