//! UR-35: links libuhd only with the feature `uhd`, from `UHD_LIB_DIR` or else
//! `pkg-config --variable=libdir uhd`; no rpath (GZ-5).

fn main() {
    println!("cargo:rerun-if-env-changed=UHD_LIB_DIR");
    if std::env::var_os("CARGO_FEATURE_UHD").is_none() {
        return;
    }
    let dir = std::env::var("UHD_LIB_DIR").ok().filter(|dir| !dir.is_empty()).or_else(|| {
        std::process::Command::new("pkg-config")
            .args(["--variable=libdir", "uhd"])
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .filter(|dir| !dir.is_empty())
    });
    let Some(dir) = dir else {
        panic!(
            "UR-35: the feature `uhd` needs libuhd: set UHD_LIB_DIR to the directory holding it, \
             or install UHD with its pkg-config file so that `pkg-config --variable=libdir uhd` finds it"
        );
    };
    println!("cargo:rustc-link-search=native={dir}");
    println!("cargo:rustc-link-lib=dylib=uhd");
}
