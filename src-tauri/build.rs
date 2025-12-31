fn main() {
    // Skip Windows resource compilation if icons don't exist
    #[cfg(not(target_os = "windows"))]
    tauri_build::build();

    #[cfg(target_os = "windows")]
    {
        // Only attempt to build if icons exist
        if std::path::Path::new("icons/icon.ico").exists() {
            tauri_build::build()
        } else {
            println!("cargo:warning=Skipping Windows resource file - icons not found");
            // Build without resource file
            tauri_build::try_build(tauri_build::Attributes::new()).unwrap_or_else(|_| {
                tauri_build::build()
            });
        }
    }
}
