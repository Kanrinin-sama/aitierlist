fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let manifest_path = std::path::Path::new(&manifest_dir);

    let commit = git_rev(manifest_path, "HEAD");
    let tree = git_rev(manifest_path, "HEAD^{tree}");

    println!("cargo:rustc-env=AITIERLIST_COMMIT={commit}");
    println!("cargo:rustc-env=AITIERLIST_TREE={tree}");

    let mut git_dir = manifest_path.join(".git");
    if git_dir.is_file() {
        println!("cargo:rerun-if-changed={}", git_dir.display());
        if let Ok(content) = std::fs::read_to_string(&git_dir)
            && let Some(path) = content
                .lines()
                .find_map(|line| line.strip_prefix("gitdir:"))
        {
            git_dir = manifest_path.join(path.trim());
        }
    }
    let common_path = git_dir.join("commondir");
    let common_dir = if let Ok(content) = std::fs::read_to_string(&common_path) {
        println!("cargo:rerun-if-changed={}", common_path.display());
        git_dir.join(content.trim())
    } else {
        git_dir.clone()
    };
    let packed_refs = common_dir.join("packed-refs");
    if packed_refs.exists() {
        println!("cargo:rerun-if-changed={}", packed_refs.display());
    }
    let head_path = git_dir.join("HEAD");
    if head_path.exists() {
        println!("cargo:rerun-if-changed={}", head_path.display());
        if let Ok(head_content) = std::fs::read_to_string(&head_path)
            && let Some(ref_path) = head_content.trim().strip_prefix("ref: ")
        {
            let mut resolved_ref = git_dir.join(ref_path.trim());
            if !resolved_ref.exists() {
                resolved_ref = common_dir.join(ref_path.trim());
            }
            if resolved_ref.exists() {
                println!("cargo:rerun-if-changed={}", resolved_ref.display());
            }
        }
    }

    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }
}

fn git_rev(manifest_dir: &std::path::Path, arg: &str) -> String {
    let fallback = "0".repeat(40);
    let Ok(output) = std::process::Command::new("git")
        .current_dir(manifest_dir)
        .args(["rev-parse", arg])
        .output()
    else {
        return fallback;
    };
    if !output.status.success() {
        return fallback;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        s
    } else {
        fallback
    }
}
