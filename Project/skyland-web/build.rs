//! 构建信息注入 — 版本构建时间与 git 分支（状态面板展示）
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    println!("cargo:rustc-env=SKYLAND_BUILD_TIME={}", secs);
    // git 分支：从仓库 .git/HEAD 读取（manifest = Project/skyland-web → 仓库根在 ../../）
    let head = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.git/HEAD");
    let branch = std::fs::read_to_string(&head)
        .map(|h| h.trim().trim_start_matches("ref: refs/heads/").trim_start_matches("ref: refs/").to_string())
        .unwrap_or_else(|_| "unknown".into());
    println!("cargo:rustc-env=SKYLAND_GIT_BRANCH={}", branch);
    println!("cargo:rerun-if-changed=build.rs");
}
