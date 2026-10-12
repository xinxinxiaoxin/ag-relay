# 网关核心维护说明

对应工单 #20、规格 #19 与 [ADR-0004](adr/0004-owned-gateway-core.md)。**网关核心**和**桌面面板**在 AG Relay 同仓配套开发、测试、发布；核心不是外部项目的只读镜像，也不以整体覆盖外部脚本作为常规升级方式。接管维护不改变现有运行行为，不要求重写核心或合并进程。

## 维护基线与职责

既有核心来源为 `xinxinxiaoxin/relay-retry-proxy` 的 `28b764d` 快照。已发布 v0.2.0 的脚本哈希与产物记录保留在 [README 的打包快照](../README.md#当前打包快照)，用于历史追溯，不代表必须跟随外部项目版本。此后以 AG Relay 本仓提交为演进基线。

| 本仓文件 | 维护职责 |
| --- | --- |
| `src-tauri/resources/gateway/relay-retry-proxy.mjs` | 请求转发、重试、流式响应、本地控制端点与诊断记录 |
| `src-tauri/resources/gateway/relay-supervisor.mjs` | 托管核心子进程与重启协作 |
| `src-tauri/resources/gateway/relay-start-check.mjs` | 启动前端口检查 |
| `src-tauri/resources/gateway/bounded-file-log.mjs` | 核心自有普通文件日志（`PROXY_LOG_FILE`）的有界轮转实现；被 `relay-retry-proxy.mjs` 导入，无端口/启动副作用 |
| `src-tauri/resources/gateway/gateway-config.bat` | 入库的环境变量默认值契约；不含密钥，不要求用户执行它来启动桌面面板 |
| `src-tauri/src/gateway_env.rs` | 桌面面板启动核心时的环境装配，与默认值契约配套维护 |
| `src-tauri/src/gateway_process.rs`、`gateway_control.rs`、`gateway_status.rs` | 实例路径、生命周期、控制请求与状态展示契约 |
| `src-tauri/src/gateway_diagnostics.rs`、`gateway_diagnostics_reader.rs`、`gateway_activity.rs` | 诊断记录到中文活动事件的映射、增量读取/分页查询，以及文件通知驱动的实时活动订阅 |
| `src-tauri/src/connection_settings.rs`、`gateway_connection.rs` | 连接设置持久化（原子替换）、环境装配与显式重启应用 |
| `src-tauri/src/bounded_log.rs` | 壳侧有界日志（`shell-exit.log` 的容量与保留） |
| `src-tauri/src/app_update.rs`、`update_journal.rs`、`atomic_write.rs` | 应用内更新的安全停机、恢复记录与同目录原子替换 |
| `src-tauri/tests/fixtures/` | 已有诊断样本与生命周期桩，随契约保留和维护 |

控制请求由桌面面板自身发起，无需为核心维护再引入外部项目的 `relay-control.mjs` 或启动 bat/ps1。ADR-0002 中已标明失效的「上游脚本零改动」理由仅作历史记录；其诊断字段约束、中文展示职责和未知事件诚实回退原则仍有效。

## 一次核心变更的流程

1. 在本仓工单中明确要改变的行为及兼容边界；先复用已确认的 Rust 应用服务层测试接缝，再修改核心。纯维护文档变更可复跑既有契约，不为文案制造无意义红测试。
2. 最小化修改本仓核心，不能以外部文件覆盖抹掉本仓改动。按变更类型同步维护：
   - **配置**：检查核心对环境变量的解释，保持 `gateway-config.bat` 和 `gateway_env_pairs` 的应用默认配置一致；核心未设置环境变量时的内置默认值可能不同，不能因接管维护就将它们改成一样。用户连接设置属于覆盖值，不得混成新的默认值。
   - **控制与状态**：生命周期、状态字段及重启请求与面板服务层一起验证。HTTP `202 Accepted` 仅表示重启已受理，不证明排空或重启已经完成。
   - **诊断**：新增或修改事件码、封闭子枚举时同步更新字典、样本与契约清单，未知码仍诚实回退。诊断记录不落盘自由文本报错、请求内容或密钥；中文活动事件由桌面面板侧提炼。不得通过删测试、减事件清单或放宽断言绕过兼容检查。
3. 运行下面的隔离契约基线；涉及真实核心行为时，在独立 Windows 沙箱或 VM 补做对应验收。已有测试不等于覆盖全部转发、重试或流式场景，按工单补充行为证据。
4. 核心与面板使用同一 AG Relay 版本构建发布，不单独在线更新核心。实际构建后记录**本仓提交**、四个核心 `.mjs`（`relay-retry-proxy.mjs`、`relay-supervisor.mjs`、`relay-start-check.mjs`、`bounded-file-log.mjs`）与配置文件的 SHA-256、成品哈希及构建时间；四个脚本与配置文件一样逐份纳入发布快照，任一改动后重打包都刷新其哈希。`loadedVersion` 是主核心脚本哈希，不等同于产品版本号。没有重打包就保留上一发布记录，不能更新日期来伪装新产物。README 的打包快照表保留的是已发布 **v0.2.0** 的历史记录（早于 `bounded-file-log.mjs` 引入），不要往该历史快照里回填新脚本的哈希；下一次实际构建时按上述口径新增记录。

## 现有契约基线与安全运行

以下测试来自规格 #19 已确认的 Rust 应用服务层接缝，不新增测试框架：

| 测试模块 | 能证明的契约 | 不能替代的验收 |
| --- | --- | --- |
| `gateway_env::tests` | 当前 17 项默认配置与入库 bat 一致、追加诊断路径且不替 supervisor 设置 `PROXY_SUPERVISED` | 真实核心启动后设置生效 |
| `gateway_diagnostics::tests` | 打包核心当前 23 个事件码、子枚举的字典覆盖，真实形状样本的译文/级别与未知码回退 | 核心真实写入到面板显示的贯通链路 |
| `gateway_diagnostics_reader::tests` | 临时诊断文件的增量读取、未完成尾行、轮转补读、分页、筛选与搜索 | 成品窗口里的实时通知与交互 |
| `gateway_activity::tests` | 临时目录 + 文件通知（notify）驱动的实时活动订阅、归属撤销与错误恢复；不触网关进程 | 成品窗口的实时追新与冻结交互 |
| `gateway_control::tests` | 持有监听器的随机本机端口假服务验证重启请求、控制头、2xx 受理、异常/超时；临时日志目录 | 真实优雅排空、重启完成；释放端口后的连接拒绝用例不在本机白名单内 |
| `gateway_status::tests` | 持有监听器的随机本机端口假服务与纯解析验证状态、字段回退及实例归属 | 真实实例运行、停止或外部进程控制；释放端口及任意 pid 父链用例不在本机白名单内 |
| `app_update::tests`、`update_commands_tests` | 更新闸门状态机、下载/验签失败拒绝、排空超时恢复、恢复记录（临时目录 + 假网关适配器，不联网、不启核心） | 真实签名新版安装与 Windows 适配器交接 |
| `bounded_log::tests`、`app_exit::tests`、`timeutil::tests`、`tray::tests` | 壳侧日志有界/轮转、退出来源格式、时间格式化与托盘动作映射等纯函数 | 真实退出流程与托盘菜单交互 |

隔离集成测试（`src-tauri/tests/`，满足隔离条件时允许在本机白名单内运行）：

| 测试文件 | 能证明的契约 | 隔离条件 | 不能替代的验收 |
| --- | --- | --- | --- |
| `tests/connection_settings.rs` | 连接设置原子保存、并发读取不遇半写文件、非法地址/端口拒绝且不覆盖旧值、环境注入只覆盖连接项 | 仅自造临时目录，不读写生产 `%APPDATA%` | 真实面板保存/重启交互 |
| `tests/gateway_connection.rs` | 拉起**随包 node 核心**的完整安全排空/停机/重启/连接切换链路 | 临时目录、动态随机端口、假上游监听器、假凭据 `fixture-only-not-a-key`、白名单环境 | 真实成品安装、升级与签名验收（见验收清单） |

上述数量是接管时的契约基线，不是永远禁止变更的上限；后续修改事件或默认配置时须同步更新独立真值和面板适配，解释变化原因。测试名称保留历史来源用语，不再表示只能从外部覆盖更新。

在**自己的工作树根目录**使用 PowerShell 运行白名单（前置 Rust 工具链已就绪；target 应指向非现用程序目录的构建缓存，并按协调约定串行、`-j 2`、`--test-threads=1`）：

```powershell
$env:CARGO_TARGET_DIR = Join-Path $PWD 'src-tauri/target'
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline -j 2 --lib gateway_env::tests -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline -j 2 --lib gateway_diagnostics::tests -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline -j 2 --lib gateway_diagnostics_reader::tests -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline -j 2 --lib gateway_activity::tests -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline -j 2 --lib app_update::tests -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline -j 2 --lib gateway_control::tests -- --test-threads=1 --skip '重启_网关未运行_连接拒绝应秒回NotRunning'
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline -j 2 --lib gateway_status::tests -- --test-threads=1 --skip '接线_端点可达但非受管且父链不可核实_归属无' --skip 'fetch_连接被拒_报错' --skip '命令体_连接被拒_离线快照' --skip '接线_端点不可达_受管句柄仍判当前窗口_否则无'
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline -j 2 --test connection_settings -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline -j 2 --test gateway_connection -- --test-threads=1
```

逐条确认退出码与测试结果，任一失败即停止后续构建；离线缺依赖则报告阻塞，不自动安装或升级工具链。并行开发遵守协调约定的 target 路径和构建限额。上述 `--skip` 排除先释放端口再请求（端口可能被别的程序接管）和查询任意真实 pid 父链的用例。其余白名单仅使用仓库样本、临时目录、纯解析或自己持有的随机监听器；若相关测试后续变化，执行前重新检查隔离条件。`gateway_connection` 会真正拉起随包核心子进程，但只在自造临时目录、动态非默认端口与假上游下运行，且只清理自己启动并保有句柄的子进程。

**不要在仍依赖现用网关的机器上直接运行无过滤的 `cargo test`、旧 `scripts/smoke.ps1`、`tauri dev` 或成品安装/启动。** 以下用例即使「只是测试」也禁止在承载现用实例的机器上运行，理由如下：

- `gateway_process::tests`：含固定 `8787` 的端口占用/真网关冒烟用例（最长等待约 120 秒并反复绑定默认端口），以及广播 `CTRL_C` / 强杀的生命周期桩——会碰到现用网关正在使用的默认端口与进程树，或对非本测试创建的进程发信号。
- `autostart::tests`：虽用临时前缀，但**仍是真实 HKCU 注册表写入**并创建/删除 `.lnk`，异常时可能残留，属系统级副作用。
- 任何需先停止现用网关、释放 `8787`、清理真实 AppData 或强杀进程才能「通过」的用例。

不能为让测试通过而停止现用网关、释放其端口或清理真实 AppData。

全量生命周期与成品验收须先确认端口、进程、应用数据和自启状态完整隔离，使用临时目录、独立测试进程、随机本机端口及假上游服务；安装/升级只在独立 Windows 沙箱或 VM。报告必须区分「契约通过」「真实成品通过」与「未执行」，不得将 HTTP mock 或浏览器假数据的通过当作真实起停、重启、诊断展示或安装更新通过。

## 吸收外部改进时的来源与许可

- 先审查外部改进与本仓的差异，再按需择取补丁或在本仓重实现；不恢复整目录覆盖流程。
- 在对应工单和提交说明中记录**来源仓库、完整提交 SHA、涉及文件/改动范围、吸收理由及本仓调整**，使变更可追溯。提交中不得携带外部环境路径、密钥或运行日志。
- 在纳入前核实该来源版本的许可证或权利人授权；保留要求的版权声明、许可证文本和归属说明。未找到许可不等于可自由使用，权利不明时先暂停纳入。
- 遵守 [ADR-0007](adr/0007-mit-open-source-distribution.md)：AG Relay 自有代码采用 MIT 的方向不改变第三方组件各自许可；涉及第三方内容时同步维护许可清单、声明与随包材料。许可材料整理见工单 #26，公开前核查见 #27，不能把核心自主维护当作已完成许可核查。
- 不改变 `relay-retry-proxy` 仓库的可见性；软件发布来源是 AG Relay，不能将请求转发的**上游服务**、外部源码来源与 AG Relay 更新源混称为同一个「上游」。
