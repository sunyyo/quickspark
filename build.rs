fn main() {
    if cfg!(target_os = "windows") {
        println!("cargo:rerun-if-changed=assets/quickspark.ico");
        let version = format!("{}.0", env!("CARGO_PKG_VERSION"));
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("assets/quickspark.ico");
        resource.set("ProductName", "QuickSpark");
        resource.set("FileVersion", &version);
        resource.set("ProductVersion", &version);
        resource.set(
            "FileDescription",
            "QuickSpark keyboard application launcher",
        );
        resource.set("OriginalFilename", "QuickSpark.exe");
        resource
            .compile()
            .expect("failed to embed application icon");
    }
}
