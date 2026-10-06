use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(dsee_helper)");
    println!("cargo:rerun-if-changed=dsee/dsee_hx_stream.cpp");
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") {
        return;
    }
    match compile_helper() {
        Ok(path) => {
            let rendered = path.to_string_lossy().replace('\\', "/");
            println!("cargo:rustc-cfg=dsee_helper");
            println!("cargo:rustc-env=DSEE_HELPER_PATH={rendered}");
        }
        Err(error) => {
            println!("cargo:warning=DSEE HX helper was not built: {error}");
        }
    }
}

fn compile_helper() -> Result<PathBuf, String> {
    let compiler = find_x86_compiler()?;
    let sdk = find_windows_sdk()?;
    let msvc_root = compiler
        .ancestors()
        .nth(4)
        .ok_or("unexpected MSVC compiler layout")?
        .to_path_buf();
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").map_err(|error| error.to_string())?);
    let source = PathBuf::from("dsee/dsee_hx_stream.cpp");
    let output = out_dir.join("dsee_hx_stream.exe");
    // Include is .../Include/<version>/{ucrt,shared,um}. Lib is .../Lib/<version>/{ucrt,um}/x86.
    let version = sdk
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Windows SDK version is not valid Unicode")?;
    let kits = sdk
        .parent()
        .and_then(Path::parent)
        .ok_or("Windows SDK Include directory is incomplete")?;
    let include = format!(
        "{};{};{};{}",
        msvc_root.join("include").display(),
        sdk.join("ucrt").display(),
        sdk.join("shared").display(),
        sdk.join("um").display()
    );
    let lib = format!(
        "{};{};{}",
        msvc_root.join("lib").join("x86").display(),
        kits.join("Lib")
            .join(version)
            .join("ucrt")
            .join("x86")
            .display(),
        kits.join("Lib")
            .join(version)
            .join("um")
            .join("x86")
            .display()
    );
    let output = Command::new(&compiler)
        .env("INCLUDE", &include)
        .env("LIB", &lib)
        .arg("/nologo")
        .arg("/EHsc")
        .arg("/MT")
        .arg("/O2")
        .arg("/std:c++17")
        .arg("/utf-8")
        .arg("/DUNICODE")
        .arg("/D_UNICODE")
        .arg(&source)
        .arg(format!("/Fe:{}", output.display()))
        .arg(format!(
            "/Fo:{}",
            out_dir.join("dsee_hx_stream.obj").display()
        ))
        .arg("/link")
        .arg("ole32.lib")
        .arg("strmiids.lib")
        .arg("quartz.lib")
        .arg("user32.lib")
        .output()
        .map_err(|error| format!("failed to start {}: {error}", compiler.display()))?;
    if !output.status.success() {
        let details = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "cl.exe exited with {}: {stdout}{details}",
            output.status
        ));
    }
    let output = out_dir.join("dsee_hx_stream.exe");
    if !output.is_file() {
        return Err("cl.exe reported success but did not write the helper".into());
    }
    Ok(output)
}

fn find_x86_compiler() -> Result<PathBuf, String> {
    let known = PathBuf::from(
        r"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x86\cl.exe",
    );
    if known.is_file() {
        return Ok(known);
    }
    let vswhere =
        PathBuf::from(r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe");
    if !vswhere.is_file() {
        return Err("Visual Studio C++ x86 compiler was not found".into());
    }
    let output = Command::new(vswhere)
        .args([
            "-latest",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("vswhere could not locate Visual C++".into());
    }
    let install = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if install.is_empty() {
        return Err("Visual C++ is not installed".into());
    }
    let tools = PathBuf::from(&install)
        .join("VC")
        .join("Tools")
        .join("MSVC");
    let mut versions = std::fs::read_dir(&tools)
        .map_err(|error| format!("cannot read {}: {error}", tools.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    versions.sort();
    let latest = versions.last().ok_or("MSVC toolset directory is empty")?;
    let compiler = latest
        .join("bin")
        .join("Hostx64")
        .join("x86")
        .join("cl.exe");
    if compiler.is_file() {
        Ok(compiler)
    } else {
        Err(format!(
            "x86 cl.exe was not found under {}",
            latest.display()
        ))
    }
}

fn find_windows_sdk() -> Result<PathBuf, String> {
    let include = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Include");
    let mut versions = std::fs::read_dir(&include)
        .map_err(|error| format!("Windows SDK was not found: {error}"))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.join("um").join("dshow.h").is_file())
        .collect::<Vec<_>>();
    versions.sort();
    versions
        .pop()
        .ok_or_else(|| "Windows SDK dshow.h was not found".into())
}
