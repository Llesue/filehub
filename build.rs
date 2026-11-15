use std::path::PathBuf;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let dist = PathBuf::from(manifest_dir).join("web/dist");

    if !dist.exists()
        && let Err(err) = std::fs::create_dir_all(&dist)
    {
        println!("cargo:warning=could not create {}: {err}", dist.display());
    }

    println!("cargo:rerun-if-changed=web/dist");
}
