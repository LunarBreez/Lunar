//! Contenu du dialogue « À propos ».
//!
//! La version est celle de ce crate, la chaîne de build celle de l'interface
//! native (Rust, gpui) ; pas de crédits nominatifs. Le seul lien est celui
//! des issues de LUNAR. LUNAR n'a pas encore de CHANGELOG.md : le menu
//! Changelog affiche « Fonction à venir ».

/// Où signaler un bug, poser une question ou demander une évolution : les
/// issues du dépôt public de LUNAR.
pub const SUPPORT_CONTACT: &str = "https://github.com/LunarBreez/Lunar/issues";

/// Version de gpui embarquée (épinglée à `=0.2.2` dans `Cargo.toml`).
pub const GPUI_VERSION: &str = "0.2.2";

/// Titre du dialogue.
pub const ABOUT_TITLE: &str = "À propos de LUNAR";

/// Présentation de LUNAR, de sa licence, de la matrice LFMS et de la
/// chaîne de build (`LUNAR_RUST_VERSION`, fournie par la CI ou lue par
/// `build.rs` dans `rust-toolchain.toml`).
pub fn about_message() -> String {
    format!(
        concat!(
            "{} {}\n\n",
            "Informations générales\n",
            "Analyseur passif de flux réseau.\n",
            "Interface native gpui.\n",
            "Licence : GNU AGPL v3.0 uniquement ({})\n",
            "Bugs, questions, demandes d'évolution : {}\n\n",
            "Chaîne de build\n",
            "Rust: {}\n",
            "gpui: {}\n",
            "Npcap (Windows): requis séparément\n\n",
            "Matrice LUNAR (LFMS)\n",
            "Le LUNAR Flow Matrix Standard est un CSV déterministe et réimportable. ",
            "Chaque ligne agrège un flux unidirectionnel identifié par ses extrémités, ",
            "protocoles et ports.\n",
            "count, total_bytes et last_seen résument l’activité ; encap_id relie les ",
            "niveaux d’un tunnel et origin trace les fichiers fusionnés."
        ),
        lunar_engine::APP_NAME,
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_LICENSE"),
        SUPPORT_CONTACT,
        option_env!("LUNAR_RUST_VERSION").unwrap_or("unknown"),
        GPUI_VERSION,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_licence_et_contact() {
        let about = about_message();
        assert!(about.starts_with(&format!("LUNAR {}", env!("CARGO_PKG_VERSION"))));
        assert!(about.contains("GNU AGPL v3.0 uniquement (AGPL-3.0-only)"));
        assert!(about.contains(
            "Bugs, questions, demandes d'évolution : https://github.com/LunarBreez/Lunar/issues\n"
        ));
        assert!(about.contains("gpui: 0.2.2"));
        assert!(
            about.contains("Rust: 1."),
            "version de Rust non injectée par build.rs"
        );
        assert!(about.contains("Matrice LUNAR (LFMS)"));
    }

    #[test]
    fn un_seul_lien_celui_des_issues() {
        let about = about_message();
        assert_eq!(
            about.matches("http").count(),
            1,
            "seul le lien des issues est attendu"
        );
        assert!(
            !about.to_lowercase().contains("dépôt"),
            "ligne Dépôt affichée"
        );
    }
}
