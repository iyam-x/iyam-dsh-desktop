use std::env;
use std::fs;
use std::path::Path;

fn main() {
    // 关键：必须先执行 tauri-build，生成 ACL manifest / capabilities 并嵌入二进制。
    // 缺失会导致所有核心权限（event/window 等）报 "Plugin not found"。
    tauri_build::build();

    // Tell cargo to rerun this script when bundled resources change
    println!("cargo:rerun-if-changed=bin/dsh-package");
    println!("cargo:rerun-if-changed=bin/dsh-shell-plugin");
    println!("cargo:rerun-if-changed=bin/node");
    println!("cargo:rerun-if-changed=tauri.conf.json");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).parent().unwrap()
        .parent().unwrap()
        .parent().unwrap()  // target/{debug|release}/
        .join("dsh-package");

    let src_dir = Path::new("bin/dsh-package");
    if src_dir.exists() {
        println!("cargo:warning=Bundling DSH package into app resources ({})",
            du_size(src_dir));
        copy_dir_all(src_dir, &dest_path).unwrap_or_else(|e| {
            panic!("Failed to copy dsh-package to app resources: {}", e);
        });
    } else {
        println!("cargo:warning=WARNING: bin/dsh-package not found, DSH will not be bundled");
    }

    // 桌面壳 companion 插件（注入 DSH 布局 CSS）
    let shell_src = Path::new("bin/dsh-shell-plugin");
    if shell_src.exists() {
        println!("cargo:warning=Bundling shell plugin into app resources");
        copy_dir_all(shell_src, &dest_path.parent().unwrap().join("dsh-shell-plugin"))
            .unwrap_or_else(|e| panic!("Failed to copy shell plugin to app resources: {}", e));
    } else {
        println!("cargo:warning=WARNING: bin/dsh-shell-plugin not found, shell plugin will not be bundled");
    }

    // 只复制当前编译目标平台的 node 运行时
    let node_target = rust_target_to_node(env::var("TARGET").unwrap_or_default());
    let node_dir = Path::new("bin/node").join(&node_target);
    let node_dest = dest_path.parent().unwrap().join("node").join(&node_target);
    if node_dir.join("node").exists() || node_dir.join("node.exe").exists() {
        println!("cargo:warning=Bundling Node runtime ({}) into app resources", node_target);
        copy_dir_all(&node_dir, &node_dest).unwrap_or_else(|e| {
            panic!("Failed to copy node runtime to app resources: {}", e);
        });
    } else {
        println!("cargo:warning=WARNING: bin/node/{node_target} not found, run `pnpm fetch:node` first");
    }
}

/// Rust target triple → scripts/fetch-node.mjs 的平台目录名
fn rust_target_to_node(target: String) -> String {
    match target.as_str() {
        "aarch64-apple-darwin" => "darwin-arm64",
        "x86_64-apple-darwin" => "darwin-x64",
        "x86_64-pc-windows-msvc" => "win32-x64",
        "aarch64-pc-windows-msvc" => "win32-arm64",
        "x86_64-unknown-linux-gnu" => "linux-x64",
        "aarch64-unknown-linux-gnu" => "linux-arm64",
        _ => target.as_str(),
    }
    .to_string()
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let name = entry.file_name();
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(name))?;
        } else {
            fs::copy(&entry.path(), &dst.join(name))?;
        }
    }
    Ok(())
}

fn du_size(path: &Path) -> String {
    let output = std::process::Command::new("du")
        .args(["-sh", path.to_str().unwrap()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok());
    output.unwrap_or_else(|| "?".to_string())
}
