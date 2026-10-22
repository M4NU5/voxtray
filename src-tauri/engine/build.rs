fn main() {
    // onnxruntime's static libs ask for `PathCch.lib`. Windows doesn't mind
    // the case, but the Windows SDK as cargo-xwin unpacks it on Linux only
    // has `pathcch.lib`. Link it under that name instead.
    let kokoro = std::env::var_os("CARGO_FEATURE_KOKORO").is_some();
    if kokoro && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/NODEFAULTLIB:PathCch.lib");
        println!("cargo:rustc-link-lib=dylib=pathcch");
    }
}
