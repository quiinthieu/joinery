#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(not(feature = "native-smoke"))]
    if std::env::var_os("JOINERY_NATIVE_SMOKE_ROOT").is_some() {
        eprintln!("Build the native-smoke feature before running the hidden native test");
        std::process::exit(2);
    }
    joinery_core::run();
}
