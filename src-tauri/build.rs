fn main() {
    tauri_build::build();
    // Chromium (CEF) : libcef.so et ses fichiers sont à côté de l'exécutable (compilation : cef-dll-sys les y copie),
    // ou dans ../lib/Lumitype une fois installé (paquets : /usr/bin/lumitype et /usr/lib/Lumitype/). L'exécutable les
    // cherche aux deux endroits, sans réglage de LD_LIBRARY_PATH (voir aussi `cef_files_switches` dans rt.rs).
    if std::env::var_os("CARGO_FEATURE_CEF").is_some() && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN:$ORIGIN/../lib/Lumitype");
    }
}
