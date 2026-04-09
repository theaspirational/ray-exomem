use std::{env, path::PathBuf, process::Command};

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

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    // -----------------------------------------------------------------------
    // 1. Build SvelteKit UI (npm run build)
    // -----------------------------------------------------------------------
    let ui_dir = manifest_dir.join("ui");

    println!("cargo:rerun-if-changed={}", ui_dir.join("src").display());
    println!("cargo:rerun-if-changed={}", ui_dir.join("package.json").display());
    println!("cargo:rerun-if-changed={}", ui_dir.join("svelte.config.js").display());
    println!("cargo:rerun-if-changed={}", ui_dir.join("vite.config.ts").display());

    if !ui_dir.join("node_modules").exists() {
        eprintln!("[build.rs] npm install in ui/");
        run("npm", &["install"], &ui_dir, "UI deps");
    }

    eprintln!("[build.rs] npm run build in ui/");
    run("npm", &["run", "build"], &ui_dir, "UI build");
    assert!(
        ui_dir.join("build/index.html").exists(),
        "UI build did not produce ui/build/index.html"
    );

    // -----------------------------------------------------------------------
    // 2. Locate or clone rayforce2
    // -----------------------------------------------------------------------
    let rayforce2_dir = env::var("RAYFORCE2_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let sibling = manifest_dir.join("../rayforce2");
            if sibling.join("Makefile").exists() {
                return sibling;
            }

            // Auto-clone into OUT_DIR for cargo install
            let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
            let cloned = out_dir.join("rayforce2");
            if !cloned.join("Makefile").exists() {
                eprintln!("[build.rs] cloning rayforce2...");
                let _ = std::fs::remove_dir_all(&cloned);
                let status = Command::new("git")
                    .args([
                        "clone", "--depth", "1",
                        "https://github.com/theaspirational/rayforce2.git",
                        "--branch", "feature/datalog-provenance",
                        cloned.to_str().unwrap(),
                    ])
                    .status()
                    .expect("failed to run git clone — is git installed?");
                if !status.success() {
                    panic!(
                        "rayforce2 not found at ../rayforce2 and git clone failed.\n\
                         Either:\n  \
                         - Check out rayforce2 alongside this repo, or\n  \
                         - Set RAYFORCE2_DIR=/path/to/rayforce2"
                    );
                }
            }
            cloned
        });

    println!("cargo:rerun-if-env-changed=RAYFORCE2_DIR");
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
        println!("cargo:rerun-if-changed={}", rayforce2_dir.join(path).display());
    }

    // -----------------------------------------------------------------------
    // 3. Build rayforce2 C library
    // -----------------------------------------------------------------------
    run("make", &["lib"], &rayforce2_dir, "rayforce2 build");

    println!("cargo:rustc-link-search=native={}", rayforce2_dir.display());
    println!("cargo:rustc-link-lib=static=rayforce");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-link-lib=pthread");
}
