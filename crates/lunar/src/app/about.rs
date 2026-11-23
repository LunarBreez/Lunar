//! Contenu du dialogue « À propos ».
//!
//! La version est celle de ce crate, la chaîne de build celle de l'interface
//! native (Rust, gpui) ; pas de crédits nominatifs. Le seul lien est celui
//! des issues de LUNAR. Le menu Changelog affiche les notes de la dernière
//! version, tirées du `CHANGELOG.md` de la racine du dépôt.

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

/// `CHANGELOG.md` de la racine du dépôt, embarqué à la compilation.
const CHANGELOG: &str = include_str!("../../../../CHANGELOG.md");

/// En-tête d'une version dans `CHANGELOG.md`, en début de ligne :
/// `## **[x.y.z] - date**`. La section « Non publié » et les sous-titres ne
/// portent pas ce motif.
const RELEASE_HEADER: &str = "\n## **[";

/// Titre du dialogue du changelog.
pub const CHANGELOG_TITLE: &str = "Changelog — LUNAR";

/// Notes de la dernière version publiée, prêtes pour un dialogue.
pub fn changelog_message() -> String {
    latest_release_notes(CHANGELOG)
}

/// Extrait de `changelog` la version la plus récente, de son en-tête à
/// l'en-tête de la version précédente.
fn latest_release_notes(changelog: &str) -> String {
    let Some(start) = changelog.find(RELEASE_HEADER) else {
        return "Changelog indisponible".to_string();
    };
    let latest = &changelog[start + 1..];
    let end = latest.find(RELEASE_HEADER).unwrap_or(latest.len());
    plain_text(latest[..end].trim_end())
}

/// Allège le markdown pour un dialogue : retire les marqueurs de titre, de
/// gras et de code, et recolle les lignes de suite d'un paragraphe ou d'une
/// puce pour que le dialogue fasse lui-même les retours à la ligne.
fn plain_text(markdown: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut joinable = false;
    for raw in markdown.lines() {
        let line = raw
            .trim_start_matches('#')
            .trim()
            .replace("**", "")
            .replace('`', "");
        let starts_block = raw.starts_with('#') || raw.starts_with("- ");
        match lines.last_mut() {
            Some(previous) if joinable && !line.is_empty() && !starts_block => {
                previous.push(' ');
                previous.push_str(&line);
            }
            _ => {
                joinable = !line.is_empty() && !raw.starts_with('#');
                lines.push(line);
            }
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Garde-fou de release : `CHANGELOG.md` est mis à jour en même temps que
    /// la version du workspace.
    #[test]
    fn le_changelog_affiche_la_version_courante() {
        assert!(
            changelog_message().starts_with(&format!("[{}]", env!("CARGO_PKG_VERSION"))),
            "la dernière version de CHANGELOG.md n'est pas la {}",
            env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    fn seule_la_derniere_version_est_extraite() {
        let notes = latest_release_notes(concat!(
            "# Changelog\n\nFormat `## **[X.Y.Z] - date**`.\n\n## Non publié\n\n- brouillon\n\n",
            "## **[1.2.0] - 2027-01-02**\n\n## Nouveautés\n\n",
            "- **Titre** : première ligne\n  suite de la puce `code`\n- autre puce\n\n",
            "## **[1.1.0] - 2027-01-01**\n\n- ancienne\n",
        ));
        assert_eq!(
            notes,
            concat!(
                "[1.2.0] - 2027-01-02\n\nNouveautés\n\n",
                "- Titre : première ligne suite de la puce code\n- autre puce"
            )
        );
    }

    #[test]
    fn changelog_sans_version() {
        assert_eq!(
            latest_release_notes("# Changelog\n\n## Non publié\n"),
            "Changelog indisponible"
        );
    }

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
