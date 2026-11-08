//! Native executable resources only. Never downloads, scans or touches profile data.
fn main() {
    println!("cargo:rerun-if-changed=assets/icons/huda.ico");
    println!("cargo:rerun-if-changed=packaging/windows/app.manifest");
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icons/huda.ico");
        // Task Manager and the Start menu show FileDescription as the app's name.
        resource.set("ProductName", "Huda");
        resource.set("FileDescription", "Huda");
        resource.set("Comments", env!("CARGO_PKG_DESCRIPTION"));
        resource.set("LegalCopyright", "Copyright (c) 2026 DevSolux");
        resource.set("CompanyName", "DevSolux");
        resource.set("ProductVersion", env!("CARGO_PKG_VERSION"));
        resource.set("FileVersion", env!("CARGO_PKG_VERSION"));
        resource.set_manifest_file("packaging/windows/app.manifest");
        resource
            .compile()
            .expect("Windows resources require the native Windows SDK resource compiler");
    }
}
