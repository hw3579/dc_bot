# Options Relay

一个基于 Tauri 2 + Vite + React + pnpm 的桌面端骨架，用来承接期权下单信号、在前端实时展示，并尽快转发到券商 API。

当前阶段已经完成：

- Tauri + Vite + pnpm 桌面应用目录初始化
- 前端控制台界面，包含原生 NATS feed 配置、多券商 relay 配置、实时状态看板
- Rust 侧 relay 状态机，支持排队、转发中、成功、失败四种状态
- Rust 侧已经直接消费 NATS subject，并把 Discord order envelope 映射到统一 relay 输入
- IBKR 适配层，默认以 dry-run 模式运行，关闭 dry-run 后会尝试通过 ibapi 下单
- Moomoo/Futu 适配层，通过 PyO3 调 Python SDK，下单前会先查 option chain 拿到期权代码

## 关于券商适配层

本次搭建时，上游公开发布的 crates.io 版本和 Git tag 还没有 3.0，当前能确认到的最新公开稳定版是 2.12.0。

当前项目固定使用 ibapi 2.12.0，并把 IB 相关代码隔离在独立 broker 模块里。新增的 Moomoo/Futu 路径则通过 PyO3 调 Python SDK，把统一的 `OptionSignalInput` 转成对应券商的下单调用。

## 启动

```bash
cd app
pnpm install
pnpm tauri dev
```

如果要在 Linux 上以前台命令行方式常驻，不拉起 Tauri 窗口：

```bash
cd app/src-tauri
cargo run -- --headless
```

headless 模式会复用同一份客户端 config JSON 和 runtime JSON；如果希望它启动后立即消费 NATS，需要确保客户端 config 里的 `autoSubscribe=true`。

## 构建单文件版本

当前默认构建目标是非 installer 的单文件可执行程序：

```bash
cd app
pnpm tauri build --no-bundle
```

构建产物位置：

- Windows: `app/src-tauri/target/release/ib-options-relay.exe`
- macOS: `app/src-tauri/target/release/ib-options-relay`

## 客户端配置与存储

客户端的 broker 配置和 NATS 订阅配置不再从仓库根目录 `.env` 读取，而是走自己的 JSON：

- client config JSON：保存 broker 配置和 NATS 配置
- runtime JSON：保存消息队列和投递结果

默认情况下，这两个文件会保存到操作系统自己的应用配置目录和数据目录；具体路径会直接显示在 Tauri 页面里。

如果你想做便携式分发，或者希望 headless relay 与某个目录绑定，可以额外设置环境变量：

- `OPTIONS_RELAY_HOME`：把 client config JSON 和 runtime JSON 都固定到这个目录

当前仍然保留的客户端环境变量只有少数 override / secret：

- `MOOMOO_TRADE_PASSWORD`
- `MOOMOO_TRADE_PASSWORD_MD5`

## 客户跟单快速配置

如果这个 Tauri app 是直接发给客户做跟单，页面右侧已经提供了一个“客户跟单快速配置”区，建议这样用：

1. 根据券商先套用 `IBKR 模拟模板` 或 `Moomoo 模拟模板`
2. 补上 NATS Server、Subject 和默认仓位 `Default Quantity`
3. 保持 `Auto Subscribe=true`、`Auto Relay=true`
4. 先用 `Dry Run` 收几条真实消息验证
5. 确认无误后点击 `一键保存跟单配置`
6. 最后再关闭 `Dry Run` 进入真实下单

## 验证

```bash
cd app
pnpm build
cargo check --manifest-path src-tauri/Cargo.toml
```

## 当前工作流

1. 页面保存 NATS server 和 subject 配置
2. Rust 进程按 `autoSubscribe` 直接连接 NATS，并监听配置好的 subject
3. 外部 feed 推送 Discord order envelope 后，Rust 会把它标准化成 `OptionSignalInput`
4. 若开启 Auto Relay，则后台异步转发到当前 broker
5. 结果写回客户端 runtime JSON；GUI 模式下同时通过事件回推到前端消息流

## 后续建议

1. 在 Python 过滤阶段补齐 `contract_type`、绝对到期日和数量，减少客户端的推断逻辑
2. 把策略消息映射为真实的 Contract/Order 规则
3. 接入 order update stream，把成交、撤单、拒单继续回写到界面
