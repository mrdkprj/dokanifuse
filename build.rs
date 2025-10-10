fn main() {
    if let Ok(env_path) = std::env::var("PATH") {
        env_path.split(";").for_each(|path| {
            println!("cargo:rustc-link-search=native={}", path);
        });
    }
    let cg = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let p = std::path::Path::new(&cg);

    let p = p.join("lib");
    println!("cargo:rustc-link-search=native={:?}", p);
    println!("cargo:rustc-link-lib=dylib=dokan2");
    println!("cargo:rustc-link-lib=dylib=dokanfuse2");
    println!("cargo:rustc-link-lib=dylib=imobiledevice-1.0");
    println!("cargo:rustc-link-lib=dylib=imobiledevice-glue-1.0");
    println!("cargo:rustc-link-lib=dylib=libcrypto");
    println!("cargo:rustc-link-lib=dylib=libssl");
    println!("cargo:rustc-link-lib=dylib=plist-2.0");
    println!("cargo:rustc-link-lib=dylib=plist++-2.0");
    println!("cargo:rustc-link-lib=dylib=usbmuxd-2.0");
}
