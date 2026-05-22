# Options Relay

一个基于 Tauri 2 + Vite + React + pnpm 的桌面端骨架，用来承接期权下单信号、在前端实时展示，并尽快转发到券商 API。

当前阶段已经完成：

- Tauri + Vite + pnpm 桌面应用目录初始化
- 前端控制台界面，包含信号录入、IB Gateway 配置、实时状态看板
- Rust 侧 relay 状态机，支持排队、转发中、成功、失败四种状态
- IBKR 适配层骨架，默认以 dry-run 模式运行，关闭 dry-run 后会尝试通过 ibapi 下单

## 关于 ibapi 3.0

本次搭建时，上游公开发布的 crates.io 版本和 Git tag 还没有 3.0，当前能确认到的最新公开稳定版是 2.12.0。

为了先把桌面端主框架搭起来，这个项目目前固定到了 ibapi 2.12.0，并把 IB 相关代码隔离在独立 relay 模块里。后续如果上游真的发布 3.0，替换成本会比较低。

## 启动

```bash
cd app
pnpm install
pnpm tauri dev
```

## 构建单文件版本

当前默认构建目标是非 installer 的单文件可执行程序：

```bash
cd app
pnpm tauri build --no-bundle
```

构建产物位置：

- Windows: `app/src-tauri/target/release/ib-options-relay.exe`
- macOS: `app/src-tauri/target/release/ib-options-relay`

## 环境变量

桌面端会在启动时读取仓库根目录的 .env，并把这些值作为 IB Gateway 面板的默认配置：

- IB_GATEWAY_HOST
- IB_GATEWAY_PORT
- IB_GATEWAY_CLIENT_ID
- IB_GATEWAY_ACCOUNT
- IB_GATEWAY_DEFAULT_EXCHANGE
- IB_GATEWAY_CURRENCY
- IB_GATEWAY_DRY_RUN
- IB_GATEWAY_AUTO_FORWARD

如果没有读到这些变量，就会回退到代码内置默认值。

## 本地 JSON 存储

应用会把当前配置和消息队列保存到 `options-relay-state.json`：

- 开发模式下保存在 `app/options-relay-state.json`
- 单文件 release 模式下保存在可执行文件所在目录

也就是说，你把可执行文件放在哪个目录运行，对应 JSON 就会写在那个目录下面。

## 验证

```bash
cd app
pnpm build
cargo check --manifest-path src-tauri/Cargo.toml
```

## 当前工作流

1. 前端录入或接收期权信号
2. Rust 侧生成本地消息并立即刷新 UI
3. 若开启 Auto Relay，则后台异步转发到 IBKR
4. 结果通过事件回推到前端消息流

## 后续建议

1. 把 Discord 导出的消息解析器接到前端表单或 Rust 命令入口
2. 把策略消息映射为真实的 Contract/Order 规则
3. 接入 order update stream，把成交、撤单、拒单继续回写到界面
