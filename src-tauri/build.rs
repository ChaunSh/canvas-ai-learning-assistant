//! Build script.
//!
//! Beyond Tauri's codegen this does one extra job: on the Windows GNU target the
//! test harness needs a Common Controls v6 manifest. `tao`/Tauri import
//! `comctl32!TaskDialogIndirect`, which only exists when comctl32 v6 is activated
//! by a manifest. Release binaries get one from `tauri-winres`; test binaries do
//! not, so without this they fail to start with
//! `STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139)`.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    tauri_build::build();

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "gnu" {
        if let Err(error) = embed_test_manifest() {
            // A missing manifest only affects `cargo test`; do not fail a release build.
            println!("cargo:warning=could not embed the test manifest: {error}");
        }
    }
}

fn embed_test_manifest() -> Result<(), String> {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").map_err(|e| e.to_string())?);
    let manifest_path = out_dir.join("caa-test.manifest");
    let rc_path = out_dir.join("caa-test.rc");
    let obj_path = out_dir.join("caa-test-manifest.o");

    std::fs::write(&manifest_path, MANIFEST).map_err(|e| e.to_string())?;
    std::fs::write(
        &rc_path,
        "1 24 \"caa-test.manifest\"\n",
    )
    .map_err(|e| e.to_string())?;

    let windres = std::env::var("WINDRES")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "windres".to_string());

    let status = Command::new(&windres)
        .current_dir(&out_dir)
        .arg("--input")
        .arg(&rc_path)
        .arg("--output")
        .arg(&obj_path)
        .arg("--output-format=coff")
        .arg("--target=pe-x86-64")
        .status()
        .map_err(|e| format!("failed to run {windres}: {e}"))?;

    if !status.success() {
        return Err(format!("windres exited with {status}"));
    }

    println!("cargo::rustc-link-arg={}", obj_path.display());
    println!("cargo:rerun-if-changed=build.rs");
    Ok(())
}

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="CanvasAIAssistant.Tests" version="1.0.0.0" processorArchitecture="*"/>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls"
                        version="6.0.0.0" processorArchitecture="*"
                        publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">permonitorv2</dpiAwareness>
    </windowsSettings>
  </application>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
</assembly>
"#;
