use std::{env, path::PathBuf, process::Command};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let rayforce2_dir = env::var("RAYFORCE2_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("../rayforce2-fork"));

    println!("cargo:rerun-if-env-changed=RAYFORCE2_DIR");
    println!(
        "cargo:rerun-if-changed={}",
        rayforce2_dir.join("Makefile").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        rayforce2_dir.join("include/rayforce.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        rayforce2_dir.join("src/core/runtime.c").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        rayforce2_dir.join("src/lang/eval.c").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        rayforce2_dir.join("src/ops/datalog.c").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        rayforce2_dir.join("src/ops/datalog.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        rayforce2_dir.join("src/lang/format.c").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        rayforce2_dir.join("src/store/splay.c").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        rayforce2_dir.join("src/store/col.c").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        rayforce2_dir.join("src/store/fileio.c").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        rayforce2_dir.join("src/table/sym.c").display()
    );

    let status = Command::new("make")
        .arg("lib")
        .current_dir(&rayforce2_dir)
        .status()
        .expect("failed to invoke `make lib` in rayforce2");

    if !status.success() {
        panic!("rayforce2 build failed");
    }

    println!("cargo:rustc-link-search=native={}", rayforce2_dir.display());
    println!("cargo:rustc-link-lib=static=rayforce");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-link-lib=pthread");
}
