# Windows 构建

## 1. 前置

- 已执行 `scripts/install-toolchain.ps1`（Rust + MinGW-w64）
- `pnpm install` 完成
- 每次新终端：`. .\scripts\env.ps1`

工具链目标：`x86_64-pc-windows-gnu`（本机无 MSVC）。

## 2. 一键构建

```powershell
. .\scripts\env.ps1
pnpm tauri:build
```

产物：

```
src-tauri/target/release/bundle/nsis/Canvas AI Learning Assistant_0.1.0_x64-setup.exe
```

> 注意：本仓库把 `CARGO_TARGET_DIR` 指向 `D:\caa-target`，
> 因此实际产物在 `D:\caa-target\release\bundle\nsis\`。
> 也可以直接看构建日志最后一行的输出路径。

## 3. 分步构建

```powershell
. .\scripts\env.ps1

# 1) 前端
pnpm build:web           # 等价于 tsc --noEmit && vite build → dist/

# 2) WebView2 加载器（GNU 目标必需）
powershell -File scripts/stage-webview2.ps1

# 3) 后端 + 打包
cd src-tauri
cargo build --release
cd ..
npx tauri build
```

### 为什么需要 `stage-webview2.ps1`

GNU 目标下 `webview2-com-sys` 以**动态**方式链接 `WebView2Loader.dll`，
而 Windows 只在 exe 所在目录（和 PATH）搜索。该 DLL 随 crate 源码分发，
脚本会把它复制到：

- `D:\caa-target\debug\` 与 `release\`
- `deps\`（测试程序需要）
- `src-tauri/webview2/`（打包为资源，安装后与 exe 同级）

缺失表现：程序启动即退出，退出码 `0xc0000139`（`STATUS_ENTRYPOINT_NOT_FOUND`）。

## 4. 打包配置

`src-tauri/tauri.conf.json` 关键项：

| 项 | 值 | 说明 |
| --- | --- | --- |
| `productName` | Canvas AI Learning Assistant | 显示名 |
| `identifier` | `com.hkustgz.canvasai` | 同时决定数据目录与 Android 包名 |
| `bundle.targets` | `["nsis"]` | Windows 安装器 |
| `bundle.windows.nsis.installMode` | `currentUser` | 无需管理员权限 |
| `bundle.windows.nsis.languages` | `SimpChinese`, `English` | 安装界面双语 |
| `app.windows[0].label` | `main` | 与命令守卫、capability 绑定 |
| `app.security.csp` | 严格策略 | 生产环境禁内联脚本 |

## 5. 安装与卸载验证

```powershell
# 静默安装到当前用户
& "D:\caa-target\release\bundle\nsis\Canvas AI Learning Assistant_0.1.0_x64-setup.exe" /S

# 检查安装位置
Get-ChildItem "$env:LOCALAPPDATA\Canvas AI Learning Assistant"

# 卸载
& "$env:LOCALAPPDATA\Canvas AI Learning Assistant\uninstall.exe" /S
```

验证要点：

1. 安装后 exe 与 `WebView2Loader.dll` 同级；
2. 首次启动能建库并渲染界面；
3. 数据目录为 `%APPDATA%\com.hkustgz.canvasai\`；
4. 卸载后用户数据仍在（不删库），如需清理请手动删除该目录。

## 6. 签名（可选）

未签名安装包会触发 SmartScreen 警告。若有代码签名证书：

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = "path/to/key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "…"
npx tauri build
```

或在 `tauri.conf.json` 的 `bundle.windows.certificateThumbprint` 指定证书指纹。

## 7. 体积与优化

`Cargo.toml` 的 release profile：

```toml
[profile.release]
lto = true
opt-level = "s"
codegen-units = 1
strip = true
panic = "abort"
```

前端已把 PDF 阅读器（pdf.js，约 1.2 MB）拆成按需加载的独立 chunk，
首屏只加载约 1.1 MB（gzip 约 350 KB）。

## 8. 常见问题

| 现象 | 处理 |
| --- | --- |
| `export ordinal too large` | 确认 `.cargo/config.toml` 的 `--exclude-all-symbols` 仍在 |
| `windres: Learning\: No such file` | 确认 `CARGO_TARGET_DIR` 指向无空格路径 |
| 打包速度极慢 | 首次需编译 500+ crate；后续增量 |
| `frontendDist not found` | 先 `pnpm build:web` |
| 安装后启动闪退 | 检查 `WebView2Loader.dll` 是否与 exe 同级；确认系统装了 WebView2 Runtime |
