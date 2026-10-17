# Android 构建

## 1. 前置

| 组件 | 版本 | 本机路径 |
| --- | --- | --- |
| JDK | 17 | `C:\Program Files\Eclipse Adoptium\jdk-17.0.20.101-hotspot` |
| Android SDK | platform 37、build-tools 36 | `%LOCALAPPDATA%\Android\Sdk` |
| NDK | 27.2.12479018 | 通过 junction 挂到 `Sdk\ndk\27.2.12479018` |
| Rust Android 目标 | 4 个 | `scripts/install-toolchain.ps1` 安装 |

`scripts/env.ps1` 会为每个目标设置链接器与 `CC_*` / `AR_*`：

```
aarch64-linux-android      → aarch64-linux-android24-clang
armv7-linux-androideabi    → armv7a-linux-androideabi24-clang
i686-linux-android         → i686-linux-android24-clang
x86_64-linux-android       → x86_64-linux-android24-clang
```

## 2. 初始化（只需一次，或改了包名/图标后）

```powershell
. .\scripts\env.ps1
npx tauri android init
```

生成 `src-tauri/gen/android/`（Gradle 工程，包含 `MainActivity.kt`）。
本仓库在该目录里额外维护了一个原生助手：
`app/src/main/java/com/hkustgz/canvasai/SecureStore.kt`（Android Keystore），
重建工程时**不要删除**它。

## 3. 构建

本机没有 Developer Mode，`tauri android build` 在把 `.so` 链接进 `jniLibs` 时会失败：

```
Creation symbolic link is not allowed for this system.
```

因此使用仓库内的脚本，它做同样的事但**复制**而不是符号链接，并在 Gradle 阶段跳过
已完成的 Rust 构建任务：

```powershell
. .\scripts\env.ps1
powershell -File scripts/build-android.ps1                 # arm64-v8a + x86_64
powershell -File scripts/build-android.ps1 -Abis arm64-v8a  # 只要 arm64
powershell -File scripts/build-android.ps1 -Release        # release 变体
```

脚本依次执行：

1. `cargo build --lib --target <abi triple> --features tauri/custom-protocol`
2. 把 `libcanvas_ai_assistant_lib.so` 复制到 `gen/android/app/src/main/jniLibs/<abi>/`
3. 把 `HTTP(S)_PROXY` 转成 Gradle 的 JVM 代理参数（访问 `dl.google.com` 需要）
4. `gradlew assembleUniversalDebug -PabiList=… -x rustBuild…`

> 如果本机已开启 Developer Mode，直接用 `npx tauri android build --apk` 也可以；
> 两种方式产物一致。

产物：

```
src-tauri/gen/android/app/build/outputs/apk/universal/
  ├── debug/app-universal-debug.apk              # 519 MiB（含调试符号，仅供调试）
  └── release/app-universal-release-unsigned.apk # 31.4 MiB（未签名）
```

> debug 包体积大的原因是 Rust 库带完整调试信息（每个 ABI 约 255 MB），
> 不影响功能，但不要用于分发。发布请用 `-Release`。

### 3.1 签名（发布前必做）

未签名的 APK 无法安装。用 build-tools 里的 `apksigner` 签名：

```powershell
$bt = "$env:LOCALAPPDATA\Android\Sdk\build-tools\36.0.0"
$apk = "src-tauri\gen\android\app\build\outputs\apk\universal\release\app-universal-release-unsigned.apk"

& "$bt\apksigner.bat" sign --ks caa-release.jks --ks-key-alias caa `
  --ks-pass pass:<storepass> --key-pass pass:<keypass> `
  --out artifacts\Canvas-AI-Learning-Assistant-0.1.0-arm64-x86_64.apk $apk

& "$bt\apksigner.bat" verify --print-certs artifacts\*.apk
```

> 仓库中的 31.48 MiB 发布包使用**临时开发密钥**签名（仅用于验证可安装性），
> 正式分发前请用自己的 keystore 重新签名。

## 4. 安装与运行

```powershell
# 查看设备
adb devices -l

# 安装
adb install -r "src-tauri\gen\android\app\build\outputs\apk\arm64\debug\app-arm64-debug.apk"

# 启动并看日志
adb shell am start -n com.hkustgz.canvasai/.MainActivity
adb logcat -s RustStdout:V RustStderr:V CaaSecureStore:V
```

### 本机限制（重要）

当前环境**没有安装 AVD 系统镜像**，也没有真机连接，因此：

- APK 可以构建，但**未在设备上运行过**；
- `docs/implementation-status.md` 中 Android 相关能力标记为 🟡「已实现·待实测」；
- 不做「已支持 Android」的宣称。

补齐验证的步骤：

```powershell
# 1) 安装系统镜像（需联网，约 1.5 GB）
sdkmanager "system-images;android-35;google_apis;x86_64"

# 2) 创建 AVD
avdmanager create avd -n caa_test -k "system-images;android-35;google_apis;x86_64" -d pixel_6

# 3) 启动（本机 HypervisorPresent=True，可用 WHPX）
emulator -avd caa_test -no-snapshot -gpu swiftshader_indirect

# 4) 安装并验证
adb install -r <apk>
```

## 5. 签名

调试包用 Android 默认 debug keystore。发布请生成自己的 keystore：

```powershell
keytool -genkeypair -v -keystore caa-release.jks -alias caa `
  -keyalg RSA -keysize 2048 -validity 10000
```

然后在 `src-tauri/gen/android/` 下配置 `keystore.properties`（**不要提交到 git**）：

```properties
storeFile=../../caa-release.jks
storePassword=…
keyAlias=caa
keyPassword=…
```

> `.gitignore` 已排除 `*.jks`、`*.keystore`、`keystore.properties`。

## 6. Android 特有实现

### 6.1 密钥存储

`SecureStore.kt`：

- 在 `AndroidKeyStore` 中生成 AES-256-GCM 密钥（`setUserAuthenticationRequired(false)`）；
- 只把 `iv || ciphertext` 的 base64 写入应用私有 SharedPreferences；
- 提供 `put` / `get` / `delete` 三个 `@JvmStatic` 方法供 JNI 调用。

Rust 侧 `src/security/android_keystore.rs`：

- 通过 `ndk_context` 拿到 `JavaVM` 与 `Context`，`init` 一次后缓存 `GlobalRef`；
- 用 `jni` 0.21 调用上述静态方法；
- 失败时返回错误（**不会**降级为明文），界面显示「保存失败」。

### 6.2 布局

- `<900px` 宽度使用顶部栏 + 底部导航（前 4 项 + 「更多」抽屉）；
- 使用了 `pt-safe` / `pb-safe` 工具类适配刘海与手势区；
- 主题色通过 `color-scheme` 同步系统深浅色。

### 6.3 文件选择

导入课件走 `@tauri-apps/plugin-dialog` 的 `open()`，在 Android 上映射到 SAF 文件选择器。
选中后由 Rust 侧读取并复制到应用私有目录（不依赖原始 URI 的长期权限）。

## 7. 常见问题

| 现象 | 处理 |
| --- | --- |
| `NDK not found` | 确认 `Sdk\ndk\27.2.12479018` 存在（junction 可用） |
| `linker 'aarch64-linux-android24-clang' not found` | 未加载 `env.ps1`，或 NDK 版本号不匹配 |
| Gradle 下载慢 | 配置代理；或先在 Android Studio 里打开一次工程 |
| `jni` 编译失败 | 确认 `Cargo.toml` 中 `jni = "0.21"`（本实现按 0.21 API 编写） |
| 启动白屏 | 检查 `frontendDist` 是否已构建；`adb logcat` 看 WebView 报错 |
| 中文显示为方块 | 设备缺中文字体，属系统问题；可在设置切成 English 验证 |
