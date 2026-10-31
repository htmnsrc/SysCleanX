// Embed the app icon and version info into the Windows executable.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(windows)]
    windows_resources();
    #[cfg(not(windows))]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        cross_resources();
    }
}

#[cfg(windows)]
fn windows_resources() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico")
        .set("ProductName", "SysCleanX")
        .set("FileDescription", "SysCleanX")
        .set("CompanyName", "Hitman")
        .set("LegalCopyright", "Copyright (c) 2025 Hitman")
        .set("OriginalFilename", "SysCleanX.exe");
    if let Err(e) = res.compile() {
        println!("cargo:warning=could not embed icon: {e}");
    }
}

/// Cross-compiling from macOS/Linux with MinGW: build a .res with windres and link it.
#[cfg(not(windows))]
fn cross_resources() {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let rc = out.join("app.rc");
    let icon = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets/icon.ico");
    let v = |k: &str| std::env::var(k).unwrap_or_else(|_| "0".into());
    let (major, minor, patch) = (v("CARGO_PKG_VERSION_MAJOR"), v("CARGO_PKG_VERSION_MINOR"), v("CARGO_PKG_VERSION_PATCH"));
    let version = v("CARGO_PKG_VERSION");
    // Icon + version details (shown in file Properties and Task Manager).
    let script = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Hitman"
      VALUE "FileDescription", "SysCleanX"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "SysCleanX"
      VALUE "LegalCopyright", "Copyright (c) 2025 Hitman"
      VALUE "OriginalFilename", "SysCleanX.exe"
      VALUE "ProductName", "SysCleanX"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = icon.display().to_string().replace('\\', "/")
    );
    std::fs::write(&rc, script).unwrap();
    let obj = out.join("app_res.o");
    let status = std::process::Command::new("x86_64-w64-mingw32-windres").arg(&rc).arg("-O").arg("coff").arg("-o").arg(&obj).status();
    match status {
        Ok(s) if s.success() => println!("cargo:rustc-link-arg-bins={}", obj.display()),
        _ => println!("cargo:warning=windres not found; exe will have no icon"),
    }
}
