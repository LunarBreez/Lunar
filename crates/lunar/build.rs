//! Script de build de LUNAR.
//!
//! Exporte `LUNAR_RUST_VERSION` (canal de `rust-toolchain.toml`), affiché
//! dans « À propos », sauf si l'environnement la fournit déjà.
//!
//! Sous Windows, lie `Packet` et `wpcap` depuis le SDK Npcap du dépôt
//! (`windows/npcap-sdk`), `wpcap.dll` en chargement différé :
//! `lunar_engine::npcap` la charge depuis `System32\Npcap` avant tout appel
//! à libpcap, et LUNAR démarre même sans Npcap ni mode WinPcap.

fn main() {
    // Racine du dépôt lue à l'exécution du script : `env!` figerait le chemin
    // de la compilation du script, que cargo réutilise après un déplacement
    // du dépôt (la version de Rust devenait alors « unknown »).
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default())
        .join("../..");
    let toolchain = root.join("rust-toolchain.toml");
    println!("cargo:rerun-if-changed={}", toolchain.display());
    println!("cargo:rerun-if-env-changed=LUNAR_RUST_VERSION");
    if std::env::var_os("LUNAR_RUST_VERSION").is_none()
        && let Some(channel) = std::fs::read_to_string(&toolchain)
            .ok()
            .as_deref()
            .and_then(toolchain_channel)
    {
        println!("cargo:rustc-env=LUNAR_RUST_VERSION={channel}");
    }

    let sdk = root.join("windows/npcap-sdk/Lib/x64");
    println!(
        "cargo:rerun-if-changed={}",
        sdk.join("Packet.lib").display()
    );
    println!("cargo:rerun-if-changed={}", sdk.join("wpcap.lib").display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-search=native={}", sdk.display());
        println!("cargo:rustc-link-lib=Packet");
        println!("cargo:rustc-link-lib=wpcap");
        println!("cargo:rustc-link-arg=/DELAYLOAD:wpcap.dll");
        println!("cargo:rustc-link-lib=delayimp");
    }
}

/// Valeur de `channel = "…"` dans `rust-toolchain.toml`.
fn toolchain_channel(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "channel").then(|| value.trim().trim_matches('"').to_string())
    })
}
