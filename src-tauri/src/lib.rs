use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
}

#[cfg_attr(feature = "desktop", tauri::command)]
fn app_info() -> AppInfo {
    AppInfo { name: "Joinery", version: env!("CARGO_PKG_VERSION") }
}

#[cfg(feature = "desktop")]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_info])
        .run(tauri::generate_context!())
        .expect("could not start Joinery");
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_version_matches_package() {
        let info = super::app_info();
        assert_eq!(info.name, "Joinery");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }
}
