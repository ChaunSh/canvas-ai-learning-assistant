# 开发指南

## 1. 环境要求

| 组件 | 版本 | 说明 |
| --- | --- | --- |
| Windows | 10/11 x64 | 本项目在 Windows 11 26300（非管理员）验证 |
| Node.js | ≥ 20 | 用于 Vite / Tauri CLI |
| pnpm | ≥ 9 | 也可用 npm，脚本以 npx 调用 |
| Rust | ≥ 1.88（实测 1.99.1） | 手动安装在 `D:\toolchain\rust` |
| MinGW-w64 GCC | ≥ 13（实测 16.2.0 UCRT） | 本机无 MSVC，使用 `x86_64-pc-windows-gnu` |
| WebView2 Runtime | 任意近期版本 | Windows 10/11 通常已内置 |
| JDK | 17 | 仅 Android 构建需要 |
| Android SDK | platform 37 / build-tools 36 | 仅 Android 构建需要 |
| NDK | 27.x | 仅 Android 构建需要 |

> 关键：**本机没有安装 MSVC**（非管理员会话无法装 Build Tools），
> 因此整条链路走 GNU 工具链。所有 `windres` / 链接器路径必须是**无空格**路径。

## 2. 首次安装工具链

```powershell
powershell -ExecutionPolicy Bypass -File scripts/install-toolchain.ps1
```

脚本行为：

1. 读取 `https://static.rust-lang.org/dist/channel-rust-stable.toml`；
2. 下载官方 `tar.xz` 包并**校验 SHA-256**；
3. 用系统 `tar` 解压后合并到 `D:\toolchain\rust`；
4. 可选安装 `rustfmt` / `clippy`；
5. 安装需要的 std：`x86_64-pc-windows-gnu` + 4 个 Android 目标。

> 为什么不直接用 rustup：本机 rustup-init 在组件下载阶段反复卡死
> （`.partial` 文件为 0 字节）。直接解包官方分发更可控。

## 3. 每次开发前

```powershell
. .\scripts\env.ps1
```

它做的三件事：

1. 设置 `RUST_HOME` / `MINGW_HOME` / `ANDROID_HOME` / `NDK_HOME` / `JAVA_HOME`；
2. 为每个 Android 目标设置 `CARGO_TARGET_*_LINKER` / `CC_*` / `AR_*`；
3. 设置 `CARGO_TARGET_DIR=D:\caa-target`（**必须**，见下）与 PATH；
4. 若检测到本机 `127.0.0.1:7890` 代理，则自动设置 `HTTPS_PROXY`。

### 为什么 `CARGO_TARGET_DIR` 必须在无空格路径

工作区路径包含空格（`Canvas Learning Assistant`）。GNU 工具链的 `windres`
在生成资源文件时不做引号处理，会报：

```
cc1.exe: fatal error: Learning\: No such file or directory
```

把 target 目录指向 `D:\caa-target` 即可绕开。

### 为什么需要 `--exclude-all-symbols`

`x86_64-pc-windows-gnu` 构建 `cdylib` 时会导出全部符号，导致链接失败：

```
ld.exe: error: export ordinal too large: 155455
```

`.cargo/config.toml` 中为该目标加了：

```toml
[target.x86_64-pc-windows-gnu]
rustflags = ["-C", "link-arg=-Wl,--exclude-all-symbols"]
```

**不要删除**，否则 Tauri 的移动端/桌面端会链接失败。

## 4. 日常命令

```powershell
. .\scripts\env.ps1

pnpm install              # 首次
pnpm tauri:dev            # 桌面开发（热更新）
pnpm build:web            # 仅构建前端（typecheck + vite build）
pnpm tauri:build          # Windows 安装包
pnpm tauri:android -- --apk   # Android APK
```

后端：

```powershell
cd src-tauri
cargo build               # 编译
cargo test                # 168 个测试
cargo clippy --all-targets
```

> 本机 `PowerShell 5.1` 下不要把 cargo 的 stderr 用管道接给 `Select-Object`：
> `$ErrorActionPreference` 会把 stderr 当终止错误，导致编译被中断。
> 推荐 `cmd /c "cargo build 1> log.txt 2>&1"`。

## 5. 目录约定

| 目录 | 内容 | 是否手改 |
| --- | --- | --- |
| `src/` | 前端源码 | ✅ |
| `src-tauri/src/` | Rust 源码 | ✅ |
| `src-tauri/migrations/` | SQL 迁移，按 `NNN_name.sql` 递增 | ✅ |
| `src-tauri/gen/android/` | Tauri 生成的 Android 工程 | 只在需要加原生代码时（如 `SecureStore.kt`） |
| `dist/` | Vite 产物，被 `frontendDist` 引用 | ❌ 自动生成 |
| `src-tauri/target/` | 未使用（已被 `CARGO_TARGET_DIR` 重定向） | ❌ |

## 6. 新增一个命令

1. 在 `src-tauri/src/commands/<域>.rs` 写函数：

```rust
#[tauri::command]
pub async fn my_command(
    window: tauri::Window,
    state: State<'_, AppState>,
    input: MyInput,
) -> AppResult<MyOutput> {
    crate::commands::guard::require_main_window(&window)?;   // 必须
    // …
}
```

2. 在 `src-tauri/src/lib.rs` 的 `invoke_handler!` 注册；
3. `src/services/api.ts` 增加对应封装；
4. `src/types/api.ts` 增加类型（与 Rust 侧同名字段，camelCase）；
5. 为纯逻辑写单元测试；跨表逻辑写集成测试。

## 7. 数据库变更

- 新增迁移文件 `migrations/002_xxx.sql`，**不要**改 `001`（已发布的迁移不可变）；
- `storage/migrations.rs` 会自动发现并顺序执行；
- 迁移必须可重复执行（`CREATE TABLE IF NOT EXISTS` / `ALTER TABLE ... ADD COLUMN`）；
- 涉及新表的查询记得补 `CREATE INDEX`。

## 8. 常见问题

| 现象 | 原因 | 处理 |
| --- | --- | --- |
| `windres` 报 `Learning\: No such file` | target 目录在含空格路径 | 确认 `CARGO_TARGET_DIR=D:\caa-target` |
| `export ordinal too large` | GNU cdylib 导出过多 | 确认 `.cargo/config.toml` 的 `--exclude-all-symbols` |
| `STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139)` 启动即退出 | 缺 `WebView2Loader.dll`（测试程序）或缺少 comctl32 v6 清单 | 运行 `scripts/stage-webview2.ps1`；`build.rs` 会为测试目标嵌入清单 |
| `ld.exe: .rsrc merge failure: multiple non-default manifests` | 二进制已由 `tauri-winres` 嵌入清单，测试目标又嵌入一个 | 无害；产物可正常运行，见 `docs/testing.md` |
| `frontendDist` 不存在 | 未构建前端 | `pnpm build:web` |
| crates 下载极慢 | 未走代理 | `scripts/env.ps1` 会自动探测 `127.0.0.1:7890` |
| Android 构建找不到 `jni` | 首次需下载 Android 目标依赖 | 保持网络/代理，重新执行一次 |
| 无法截取窗口画面 | 前台激活被系统限制 | 用 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` 走 CDP |

## 9. 代码风格

- Rust：`cargo fmt`，错误统一 `AppError`（可序列化为 `{code,message}`）；
- TypeScript：`strict` + `noUnusedLocals`，禁止 `any`；
- 注释解释**为什么**，不复述代码；
- 面向用户的文案走 i18n 字典，不硬编码在组件里（少量例外见注释）。
