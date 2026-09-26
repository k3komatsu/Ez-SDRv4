//! Links libuhd: `UHD_LIB_DIR` if set, else pkg-config's libdir, else Homebrew's.
fn main() {
    println!("cargo:rerun-if-env-changed=UHD_LIB_DIR");
    let dir = std::env::var("UHD_LIB_DIR").ok().or_else(|| {
        let out = std::process::Command::new("pkg-config")
            .args(["--variable=libdir", "uhd"])
            .output()
            .ok()?;
        let dir = String::from_utf8(out.stdout).ok()?.trim().to_owned();
        (out.status.success() && !dir.is_empty()).then_some(dir)
    });
    let dir = dir.unwrap_or_else(|| "/opt/homebrew/lib".to_owned());
    println!("cargo:rustc-link-search=native={dir}");
    println!("cargo:rustc-link-lib=dylib=uhd");
    // So the binary finds libuhd at run time without DYLD/LD_LIBRARY_PATH.
    println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
}
