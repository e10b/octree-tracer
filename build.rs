fn main() {
    println!("cargo:rerun-if-env-changed=OIDN_DIR");
    let Some(dir) = std::env::var_os("OIDN_DIR") else {
        return;
    };
    let lib = std::path::PathBuf::from(dir).join("lib");
    if lib.is_dir() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
    }
}
