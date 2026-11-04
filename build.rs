fn main() {
    println!("cargo:rerun-if-changed=assets/app/app.ico");
    println!("cargo:rerun-if-changed=build.rs");

    // A build script that can break the build is worse than an icon that is
    // missing, so *everything* below is caught: a panic inside the resource
    // helper (it reads Cargo.toml and the registry) is turned into a warning just
    // like a compile error.
    #[cfg(windows)]
    {
        let done = std::panic::catch_unwind(windows_resource);
        if done.is_err() {
            println!("cargo:warning=could not embed the application icon (the resource helper panicked)");
        }
    }
}

#[cfg(windows)]
fn windows_resource() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/app/app.ico");
    res.set("ProductName", "Bare MD");
    res.set("FileDescription", "Bare MD - Markdown editor");
    res.set("ProductVersion", env!("CARGO_PKG_VERSION"));
    res.set("FileVersion", env!("CARGO_PKG_VERSION"));
    res.set("LegalCopyright", "AGPL-3.0");
    // `compile` returns `io::Result` (`try_compile` does not exist in this
    // version), and its error is *reported*, not propagated: the icon is a nicety,
    // a build that cannot find `rc.exe` should still produce a working (iconless)
    // binary.
    if let Err(e) = res.compile() {
        println!("cargo:warning=could not embed the application icon: {e}");
    }
}
