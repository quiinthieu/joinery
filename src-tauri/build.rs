#[cfg(feature = "desktop")]
fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&["app_info"]),
        ),
    )
    .expect("Tauri build configuration must be valid");
}

#[cfg(not(feature = "desktop"))]
fn main() {}
