use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn run(cmd: &str, args: &[&str], dir: &PathBuf, label: &str) {
    let status = Command::new(cmd)
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap_or_else(|e| panic!("{label}: failed to run `{cmd}`: {e}"));
    if !status.success() {
        panic!("{label}: `{cmd} {}` failed", args.join(" "));
    }
}

fn command_output(cmd: &str, args: &[&str], dir: &PathBuf) -> Option<String> {
    let output = Command::new(cmd)
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn build_unix_timestamp() -> String {
    env::var("SOURCE_DATE_EPOCH").unwrap_or_else(|_| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before UNIX_EPOCH")
            .as_secs()
            .to_string()
    })
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let git_sha = command_output("git", &["rev-parse", "--short=12", "HEAD"], &manifest_dir)
        .unwrap_or_else(|| "unknown".to_string());
    let build_unix = build_unix_timestamp();

    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-env-changed=RAY_EXOMEM_BASE_PATH");
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join(".git/HEAD").display()
    );
    println!("cargo:rustc-env=RAY_EXOMEM_GIT_SHA={git_sha}");
    println!("cargo:rustc-env=RAY_EXOMEM_BUILD_UNIX={build_unix}");

    // Sub-path mount, baked at compile time. Default empty (root). To mount the
    // whole app under e.g. somesite.com/ray-exomem, set this env var before
    // `cargo build`. The same value is propagated to the SvelteKit build via
    // env so its baked asset paths match.
    let raw_base = env::var("RAY_EXOMEM_BASE_PATH").unwrap_or_default();
    let base_path = raw_base.trim().trim_end_matches('/').to_string();
    if !base_path.is_empty() && !base_path.starts_with('/') {
        panic!(
            "RAY_EXOMEM_BASE_PATH must start with `/` (got `{}`)",
            base_path
        );
    }
    println!("cargo:rustc-env=RAY_EXOMEM_BASE_PATH={base_path}");
    // Re-export so the SvelteKit build picks the same value out of process env.
    unsafe {
        env::set_var("RAY_EXOMEM_BASE_PATH", &base_path);
    }

    // -----------------------------------------------------------------------
    // 0. Scan bootstrap/*.json for drop-in seed fixtures.
    //
    // Each JSON file in `bootstrap/` is embedded into the binary at compile
    // time. Files are gitignored so extension developers can drop their own
    // private seeds in without committing them. An empty bootstrap/ ships a
    // binary with no seed data — that's a valid configuration.
    // -----------------------------------------------------------------------
    emit_bootstrap_seeds(&manifest_dir);

    // -----------------------------------------------------------------------
    // 1. Build SvelteKit UI (npm run build)
    // -----------------------------------------------------------------------
    let ui_dir = manifest_dir.join("ui");

    println!("cargo:rerun-if-changed={}", ui_dir.join("src").display());
    println!(
        "cargo:rerun-if-changed={}",
        ui_dir.join("package.json").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        ui_dir.join("svelte.config.js").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        ui_dir.join("vite.config.ts").display()
    );

    if !ui_dir.join("node_modules").exists() {
        eprintln!("[build.rs] npm install in ui/");
        run("npm", &["install"], &ui_dir, "UI deps");
    }

    // Wipe ui/build/ so SvelteKit's adapter-static doesn't accumulate stale
    // content-hashed chunks from prior builds. The adapter copies from
    // .svelte-kit/output into ui/build/ but never prunes orphans.
    let ui_build = ui_dir.join("build");
    if ui_build.exists() {
        fs::remove_dir_all(&ui_build).unwrap_or_else(|e| panic!("failed to clean ui/build/: {e}"));
    }

    eprintln!("[build.rs] npm run build in ui/");
    run("npm", &["run", "build"], &ui_dir, "UI build");
    assert!(
        ui_dir.join("build/index.html").exists(),
        "UI build did not produce ui/build/index.html"
    );

    // -----------------------------------------------------------------------
    // 2. Locate or clone rayforce
    // -----------------------------------------------------------------------
    let rayforce_dir = env::var("RAYFORCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let sibling = manifest_dir.join("../rayforce");
            if sibling.join("Makefile").exists() {
                return sibling;
            }

            // Auto-clone into OUT_DIR for cargo install
            let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
            let cloned = out_dir.join("rayforce");
            const RAYFORCE_REPO: &str = "https://github.com/RayforceDB/rayforce.git";
            const RAYFORCE_REF: &str = "master";
            if !cloned.join("Makefile").exists() {
                eprintln!("[build.rs] fetching rayforce {RAYFORCE_REF}...");
                let _ = std::fs::remove_dir_all(&cloned);
                std::fs::create_dir_all(&cloned).expect("failed to create rayforce clone dir");
                run("git", &["init"], &cloned, "rayforce init");
                run(
                    "git",
                    &["remote", "add", "origin", RAYFORCE_REPO],
                    &cloned,
                    "rayforce remote add",
                );
                let fetch_status = Command::new("git")
                    .args(["fetch", "--depth", "1", "origin", RAYFORCE_REF])
                    .current_dir(&cloned)
                    .status()
                    .expect("failed to run git fetch — is git installed?");
                if !fetch_status.success() {
                    panic!(
                        "rayforce not found at ../rayforce and fetching {RAYFORCE_REPO} {RAYFORCE_REF} failed.\n\
                         Either:\n  \
                         - Check out RayforceDB/rayforce alongside this repo on the same ref, or\n  \
                         - Set RAYFORCE_DIR=/path/to/rayforce"
                    );
                }
                run(
                    "git",
                    &["checkout", "--detach", "FETCH_HEAD"],
                    &cloned,
                    "rayforce checkout",
                );
            }
            cloned
        });

    println!("cargo:rerun-if-env-changed=RAYFORCE_DIR");
    for path in &[
        "Makefile",
        "include/rayforce.h",
        "src/core/runtime.c",
        "src/lang/eval.c",
        "src/ops/datalog.c",
        "src/ops/datalog.h",
        "src/lang/format.c",
        "src/store/splay.c",
        "src/store/col.c",
        "src/store/fileio.c",
        "src/table/sym.c",
    ] {
        println!(
            "cargo:rerun-if-changed={}",
            rayforce_dir.join(path).display()
        );
    }

    // -----------------------------------------------------------------------
    // 3. Build rayforce C library
    // -----------------------------------------------------------------------
    run("make", &["lib"], &rayforce_dir, "rayforce build");

    println!("cargo:rustc-link-search=native={}", rayforce_dir.display());
    println!("cargo:rustc-link-lib=static=rayforce");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-link-lib=pthread");
}

fn emit_bootstrap_seeds(manifest_dir: &Path) {
    let bootstrap_dir = manifest_dir.join("bootstrap");
    println!("cargo:rerun-if-changed={}", bootstrap_dir.display());

    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(read) = fs::read_dir(&bootstrap_dir) {
        for entry in read.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !name.ends_with(".json") || name.starts_with('.') {
                continue;
            }
            println!("cargo:rerun-if-changed={}", path.display());
            entries.push((name.to_string(), path));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut generated = String::new();
    generated.push_str(
        "// @generated by build.rs — do not edit.\n\
         pub static BOOTSTRAP_SEED_FILES: &[(&str, &str)] = &[\n",
    );
    for (name, path) in &entries {
        let escaped_name = name.replace('\\', "\\\\").replace('"', "\\\"");
        let path_str = path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        generated.push_str(&format!(
            "    (\"{escaped_name}\", include_str!(\"{path_str}\")),\n"
        ));
    }
    generated.push_str("];\n");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let out_path = out_dir.join("bootstrap_seeds.rs");
    fs::write(&out_path, generated).expect("failed to write bootstrap_seeds.rs");
}
