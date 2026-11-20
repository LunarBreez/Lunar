//! Page de synthèse exportée depuis la vue d'investigation : un document HTML
//! autonome, lisible sans LUNAR, à transmettre avec la matrice CSV qu'il
//! résume.
//!
//! La page est inerte : aucun script, aucun gestionnaire d'événement, aucune
//! ressource externe, et une politique CSP qui interdit tout chargement si le
//! navigateur l'ouvre malgré tout avec des droits plus larges. Toutes les
//! valeurs issues du relevé (labels saisis, noms, protocoles…) sont échappées.
//!
//! Elle reprend les agrégations de la vue ([`super::tables`]) sur le relevé
//! complet, sans les filtres d'affichage, et porte l'empreinte SHA-256 du CSV
//! écrit à côté par le backend : on vérifie ainsi qu'elle décrit bien ce
//! fichier. Les déductions restent des indications à confirmer : des
//! déductions bornées, jamais des conclusions.
//!
//! Pour une même entrée (et un même fuseau), la page est identique à l'octet
//! aux références de `tests/golden/`.

use std::cmp::Reverse;

use lunar_flows_core::graph::GraphData;

use super::dates::{
    EpochMillis, LocalZone, format_long_medium, format_short_medium, to_iso_string,
};
use super::glossary::match_glossary;
use super::graph_view::GraphView;
use super::locale::{compare_fr, format_integer};
use super::period::{PeriodSummary, SurveyPeriodView, duration_value, summarize_period};
use super::protocol_colors::color_for_protocol;
use super::tables::{
    Totals, build_deduction_rows, build_endpoint_rows, build_flow_rows, build_summary,
    compute_stats, format_bytes, observed_glossary_tokens, percent_of,
};

/// Ce que l'interface reçoit pour composer la page : le graphe de l'instant
/// de l'export et l'identité du CSV écrit à côté.
#[derive(Debug, Default)]
pub struct SynthesisExport {
    /// Graphe complet à l'instant de l'export (nœuds = endpoints, arêtes =
    /// flux).
    pub graph: GraphData,
    /// Nom du CSV seul, sans dossier : la page ne divulgue pas
    /// l'arborescence du poste de relevé.
    pub csv_file_name: String,
    /// SHA-256 du CSV tel qu'écrit sur le disque, en hexadécimal minuscule.
    pub csv_sha256: String,
    /// Taille du CSV écrit, en octets.
    pub csv_bytes: u64,
    /// Type de liaison du relevé (`ETHERNET`…), comme dans le préambule LFMS.
    pub link_type: Option<String>,
    /// Site du relevé, s'il a été renseigné.
    pub site: Option<String>,
    /// Capteur (poste de relevé), s'il a été renseigné.
    pub sensor: Option<String>,
    /// Interface écoutée, si elle a été renseignée.
    pub interface: Option<String>,
    /// Nom du produit écrit dans la page (« LUNAR »).
    pub app_name: String,
    /// Version du produit qui a produit le relevé exporté.
    pub app_version: String,
    /// Période observée du relevé, écrite aussi dans le préambule du CSV.
    pub period: SurveyPeriodView,
}

/// Nombre de lignes des classements « Top » de la page.
const TOP_N: usize = 10;

/// Politique de sécurité du contenu : rien ne se charge, seul le style en
/// ligne s'applique.
const CSP: &str =
    "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'";

/// Échappe une valeur pour un contenu ou un attribut HTML entre guillemets.
pub fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Vrai pour `#` suivi de 3 à 8 chiffres hexadécimaux.
fn is_hex_color(color: &str) -> bool {
    color.strip_prefix('#').is_some_and(|hex| {
        (3..=8).contains(&hex.len()) && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

/// Couleur de protocole, limitée à un code hexadécimal : rien d'autre ne
/// peut entrer dans un attribut `style`.
fn safe_color(protocol: &str) -> &'static str {
    let color = color_for_protocol(Some(protocol));
    if is_hex_color(color) {
        color
    } else {
        "#888888"
    }
}

/// Entier au format fr-FR.
fn integer(value: impl Into<u64>) -> String {
    format_integer(value.into())
}

/// Entier au format fr-FR, depuis un dénombrement.
fn count(value: usize) -> String {
    format_integer(u64::try_from(value).unwrap_or(u64::MAX))
}

/// Barre horizontale proportionnelle (`width` en % entiers, au moins 1 %
/// pour une part non nulle).
fn bar(part: u64, max: u64, color: &str) -> String {
    let width = if max > 0 && part > 0 {
        #[allow(clippy::cast_precision_loss)]
        let ratio = part as f64 / max as f64 * 100.0;
        // `Math.round` d'une valeur positive : la moitié va vers le haut.
        ratio.round().max(1.0)
    } else {
        0.0
    };
    format!(
        r#"<span class="bar"><span class="fill" style="width:{width}%;background:{color}"></span></span>"#
    )
}

/// Couleur des barres sans protocole.
const DEFAULT_BAR_COLOR: &str = "#2f6fb0";

/// Tableau HTML : en-têtes échappés, cellules déjà composées, colonnes
/// numériques alignées à droite, ligne `empty` si aucune ligne.
fn table(headers: &[&str], rows: &[Vec<String>], numeric: &[usize], empty: &str) -> String {
    let class = |index: usize| {
        if numeric.contains(&index) {
            r#" class="num""#
        } else {
            ""
        }
    };
    let head: String = headers
        .iter()
        .enumerate()
        .map(|(index, header)| format!("<th{}>{}</th>", class(index), escape_html(header)))
        .collect();
    let body = if rows.is_empty() {
        format!(
            r#"<tr><td colspan="{}" class="muted">{}</td></tr>"#,
            headers.len(),
            escape_html(empty)
        )
    } else {
        rows.iter()
            .map(|cells| {
                let cells: String = cells
                    .iter()
                    .enumerate()
                    .map(|(index, cell)| format!("<td{}>{cell}</td>", class(index)))
                    .collect();
                format!("<tr>{cells}</tr>")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!("<table>\n<thead><tr>{head}</tr></thead>\n<tbody>\n{body}\n</tbody>\n</table>")
}

/// Période observée, en heure locale du poste qui exporte, avec les bornes
/// UTC exactes du préambule du CSV joint.
fn period_text(period: &SurveyPeriodView, zone: &impl LocalZone) -> Option<String> {
    let (view_first, view_last) = match period {
        SurveyPeriodView::Unknown => {
            return Some("inconnue : une matrice importée ne déclarait pas sa période".to_string());
        }
        SurveyPeriodView::Empty => return None,
        SurveyPeriodView::Known { first, last } => (first, last),
    };
    let PeriodSummary::Known {
        first,
        last,
        duration,
    } = summarize_period(period)
    else {
        return None;
    };
    Some(format!(
        "du {} au {}, soit {duration} (UTC : {view_first} à {view_last})",
        format_short_medium(first, zone),
        format_short_medium(last, zone)
    ))
}

/// Lignes du contexte (site, capteur…) : seules les valeurs renseignées.
fn context_rows(export: &SynthesisExport, zone: &impl LocalZone) -> Vec<(&'static str, String)> {
    [
        ("Site", export.site.clone()),
        ("Capteur", export.sensor.clone()),
        ("Interface", export.interface.clone()),
        ("Type de liaison", export.link_type.clone()),
        ("Période observée", period_text(&export.period, zone)),
    ]
    .into_iter()
    .filter_map(|(key, value)| {
        value
            .filter(|value| !value.is_empty())
            .map(|value| (key, value))
    })
    .collect()
}

/// Lignes « nom, flux, volume, part, barre » des répartitions.
fn share_rows(rows: &[(&str, Totals)], total_bytes: u64) -> Vec<Vec<String>> {
    rows.iter()
        .map(|(name, totals)| {
            vec![
                escape_html(name),
                count(totals.flows),
                escape_html(&format_bytes(totals.bytes)),
                escape_html(&percent_of(totals.bytes, total_bytes)),
                bar(totals.bytes, total_bytes, DEFAULT_BAR_COLOR),
            ]
        })
        .collect()
}

/// Feuille de style de la page (aucune ressource externe).
const STYLE: &str = r#":root { color-scheme: light; }
body { margin: 0; padding: 24px 32px 48px; font: 14px/1.45 "Segoe UI", system-ui, sans-serif; color: #1c2430; background: #ffffff; }
h1 { font-size: 22px; margin: 0 0 4px; }
h2 { font-size: 16px; margin: 32px 0 10px; padding-bottom: 4px; border-bottom: 2px solid #d5dde8; }
.sub { color: #556274; margin: 0 0 16px; }
.muted { color: #6b7686; }
.warn { color: #9a4a00; font-weight: 600; }
dl.context { display: grid; grid-template-columns: max-content 1fr; gap: 2px 16px; margin: 0 0 16px; }
dl.context dt { font-weight: 600; }
dl.context dd { margin: 0; }
.fingerprint { border: 1px solid #c7d3e3; background: #f4f7fb; border-radius: 6px; padding: 12px 16px; }
.fingerprint p { margin: 4px 0; }
code, .hash { font-family: "Cascadia Mono", Consolas, "DejaVu Sans Mono", monospace; font-size: 13px; }
.hash { word-break: break-all; font-weight: 600; }
.kpis { display: flex; flex-wrap: wrap; gap: 10px; }
.kpi { border: 1px solid #d5dde8; border-radius: 6px; padding: 8px 14px; min-width: 110px; }
.kpi b { display: block; font-size: 20px; }
.kpi span { font-size: 12px; color: #556274; text-transform: uppercase; letter-spacing: .03em; }
.kpi.alert { border-color: #d9a066; background: #fff6ec; }
table { border-collapse: collapse; width: 100%; font-size: 13px; }
th, td { text-align: left; padding: 5px 8px; border-bottom: 1px solid #e3e8ef; vertical-align: top; }
th { background: #eef2f7; position: sticky; top: 0; }
td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }
.bar { display: inline-block; width: 160px; height: 8px; background: #e8edf3; border-radius: 4px; overflow: hidden; vertical-align: middle; }
.fill { display: block; height: 100%; }
.swatch { display: inline-block; width: 10px; height: 10px; border-radius: 2px; margin-right: 6px; vertical-align: middle; }
dl.glossary dt { font-weight: 600; margin-top: 10px; }
dl.glossary dd { margin: 2px 0 0; }
footer { margin-top: 40px; font-size: 12px; color: #6b7686; }
@media print { body { padding: 0; } th { position: static; } h2 { break-after: avoid; } tr { break-inside: avoid; } }
"#;

/// Compose la page de synthèse. Fonction pure : même entrée, même instant et
/// même fuseau, même page. `generated_at` est l'instant de l'export (affiché
/// dans l'en-tête), `zone` le fuseau du poste qui exporte.
pub fn render_synthesis_html(
    export: &SynthesisExport,
    generated_at: EpochMillis,
    zone: &impl LocalZone,
) -> String {
    let graph = GraphView::new(&export.graph);
    let stats = compute_stats(&graph);
    let summary = build_summary(&graph, TOP_N);
    let mut endpoints: Vec<_> = build_endpoint_rows(&graph)
        .into_iter()
        .filter(|row| row.degree > 0)
        .collect();
    endpoints.sort_by_key(|row| Reverse(row.bytes));
    let mut flows = build_flow_rows(&graph);
    flows.sort_by_key(|row| Reverse(row.bytes));
    let mut deductions = build_deduction_rows(&graph);
    deductions.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| compare_fr(&a.observed, &b.observed))
    });
    let mut glossary = match_glossary(&observed_glossary_tokens(&graph));
    glossary.sort_by(|a, b| {
        a.category
            .order_index()
            .cmp(&b.category.order_index())
            .then_with(|| compare_fr(a.term, b.term))
    });

    let generated_local = format_long_medium(Some(generated_at), zone);
    let generated_iso = to_iso_string(generated_at);
    let csv_name = escape_html(&export.csv_file_name);
    let sha = escape_html(&export.csv_sha256);
    let version = escape_html(&export.app_version);
    let app = escape_html(&export.app_name);

    let context: String = context_rows(export, zone)
        .iter()
        .map(|(key, value)| {
            format!(
                "<dt>{}</dt><dd>{}</dd>",
                escape_html(key),
                escape_html(value)
            )
        })
        .collect();

    let kpis: [(&str, String, bool); 8] = [
        ("Endpoints", count(stats.endpoints), false),
        ("Flux", count(stats.flows), false),
        ("Paquets", integer(stats.packets), false),
        ("Volume", format_bytes(stats.bytes), false),
        ("Protocoles", count(stats.protocols), false),
        (
            "IP dupliquées",
            count(stats.duplicate_ips),
            stats.duplicate_ips > 0,
        ),
        (
            "Endpoints sans label",
            count(summary.unlabeled),
            summary.unlabeled > 0,
        ),
        ("Durée du relevé", duration_value(&export.period), false),
    ];

    let max_protocol_bytes = summary.protocols.first().map_or(0, |share| share.bytes);
    let protocol_rows: Vec<Vec<String>> = summary
        .protocols
        .iter()
        .map(|share| {
            let color = safe_color(&share.protocol);
            vec![
                format!(
                    r#"<span class="swatch" style="background:{color}"></span>{}"#,
                    escape_html(&share.protocol)
                ),
                count(share.flows),
                integer(share.packets),
                escape_html(&format_bytes(share.bytes)),
                escape_html(&percent_of(share.bytes, stats.bytes)),
                bar(share.bytes, max_protocol_bytes, color),
            ]
        })
        .collect();
    let protocol_table = table(
        &[
            "Protocole",
            "Flux",
            "Paquets",
            "Volume",
            "Part du volume",
            "",
        ],
        &protocol_rows,
        &[1, 2, 3, 4],
        "Aucun flux.",
    );

    let share_headers = ["", "Flux", "Volume", "Part du volume", ""];
    let scope_table = table(
        &share_headers,
        &share_rows(
            &[
                ("Interne", summary.scope.internal),
                ("Internet", summary.scope.internet),
            ],
            stats.bytes,
        ),
        &[1, 2, 3],
        "Aucun flux.",
    );
    let confidentiality: Vec<(&str, Totals)> = summary
        .confidentiality
        .iter()
        .map(|part| (part.kind.label(), part.totals))
        .collect();
    let confidentiality_table = table(
        &share_headers,
        &share_rows(&confidentiality, stats.bytes),
        &[1, 2, 3],
        "Aucun flux.",
    );
    let usages: Vec<(&str, Totals)> = summary
        .usages
        .iter()
        .map(|part| (part.usage.label(), part.totals))
        .collect();
    let usage_table = table(
        &share_headers,
        &share_rows(&usages, stats.bytes),
        &[1, 2, 3],
        "Aucun flux.",
    );

    let max_endpoint_bytes = summary.top_endpoints.first().map_or(0, |row| row.bytes);
    let top_endpoint_rows: Vec<Vec<String>> = summary
        .top_endpoints
        .iter()
        .map(|row| {
            vec![
                escape_html(&row.name),
                escape_html(if row.ip.is_empty() { "—" } else { &row.ip }),
                count(row.degree),
                escape_html(&format_bytes(row.bytes)),
                bar(row.bytes, max_endpoint_bytes, DEFAULT_BAR_COLOR),
            ]
        })
        .collect();
    let top_endpoints = table(
        &["Endpoint", "IP", "Flux", "Volume", ""],
        &top_endpoint_rows,
        &[2, 3],
        "Aucun endpoint.",
    );

    let max_flow_bytes = summary.top_flows.first().map_or(0, |row| row.bytes);
    let top_flow_rows: Vec<Vec<String>> = summary
        .top_flows
        .iter()
        .map(|row| {
            vec![
                escape_html(&row.source),
                arrow(row.bidir).to_string(),
                escape_html(&row.target),
                escape_html(&row.protocol),
                integer(row.packets),
                escape_html(&format_bytes(row.bytes)),
                bar(row.bytes, max_flow_bytes, safe_color(&row.protocol)),
            ]
        })
        .collect();
    let top_flows = table(
        &[
            "Source",
            "",
            "Destination",
            "Protocole",
            "Paquets",
            "Volume",
            "",
        ],
        &top_flow_rows,
        &[4, 5],
        "Aucun flux.",
    );

    let endpoint_rows: Vec<Vec<String>> = endpoints
        .iter()
        .map(|row| {
            let duplicate = if row.duplicate_ip {
                r#" <span class="warn" title="IP présente sur un autre VLAN">⚠ IP dupliquée</span>"#
            } else {
                ""
            };
            let extra_macs = if row.macs.len() > 1 {
                format!(r#" <span class="muted">(+{})</span>"#, row.macs.len() - 1)
            } else {
                String::new()
            };
            vec![
                escape_html(&row.name),
                format!(
                    "{}{duplicate}",
                    escape_html(if row.ip.is_empty() { "—" } else { &row.ip })
                ),
                format!(
                    "{}{extra_macs}",
                    escape_html(if row.mac_display.is_empty() {
                        "—"
                    } else {
                        &row.mac_display
                    })
                ),
                escape_html(
                    &row.vlan
                        .map_or_else(|| "—".to_string(), |vlan| vlan.to_string()),
                ),
                escape_html(if row.label.is_empty() {
                    "—"
                } else {
                    &row.label
                }),
                count(row.degree),
                count(row.flows_out),
                count(row.flows_in),
                integer(row.packets),
                escape_html(&format_bytes(row.bytes)),
            ]
        })
        .collect();
    let endpoint_table = table(
        &[
            "Endpoint", "IP", "MAC", "VLAN", "Label", "Flux", "Sortants", "Entrants", "Paquets",
            "Volume",
        ],
        &endpoint_rows,
        &[5, 6, 7, 8, 9],
        "Aucun endpoint.",
    );

    let flow_rows: Vec<Vec<String>> = flows
        .iter()
        .map(|row| {
            vec![
                escape_html(&row.source),
                escape_html(&row.target),
                escape_html(&row.protocol),
                escape_html(if row.ports.is_empty() {
                    "—"
                } else {
                    &row.ports
                }),
                arrow(row.bidir).to_string(),
                integer(row.packets),
                escape_html(&format_bytes(row.bytes)),
            ]
        })
        .collect();
    let flow_table = table(
        &[
            "Source",
            "Destination",
            "Protocole",
            "Ports",
            "Sens",
            "Paquets",
            "Volume",
        ],
        &flow_rows,
        &[5, 6],
        "Aucun flux.",
    );

    let deduction_rows: Vec<Vec<String>> = deductions
        .iter()
        .map(|row| {
            vec![
                escape_html(row.kind.label()),
                escape_html(&row.observed),
                escape_html(row.deduction),
                count(row.count),
            ]
        })
        .collect();
    let deduction_table = table(
        &["Type", "Observé", "Indication", "Occurrences"],
        &deduction_rows,
        &[3],
        "Aucun port, fabricant ou adresse notoire reconnu dans ce relevé.",
    );

    let glossary_section = if glossary.is_empty() {
        r#"<p class="muted">Aucun terme du glossaire n'apparaît dans ce relevé.</p>"#.to_string()
    } else {
        let entries: Vec<String> = glossary
            .iter()
            .map(|entry| {
                format!(
                    r#"<dt>{} <span class="muted">· {}</span></dt><dd>{}</dd>"#,
                    escape_html(entry.term),
                    escape_html(entry.category.label()),
                    escape_html(&entry.definition_for(&export.app_name))
                )
            })
            .collect();
        format!(r#"<dl class="glossary">{}</dl>"#, entries.join("\n"))
    };

    let kpi_lines: Vec<String> = kpis
        .iter()
        .map(|(label, value, alert)| {
            format!(
                r#"<div class="kpi{}"><b>{}</b><span>{}</span></div>"#,
                if *alert { " alert" } else { "" },
                escape_html(value),
                escape_html(label)
            )
        })
        .collect();
    let generated_local = escape_html(&generated_local);
    let generated_iso = escape_html(&generated_iso);
    let context_block = if context.is_empty() {
        String::new()
    } else {
        format!(r#"<dl class="context">{context}</dl>"#)
    };

    let mut html = String::with_capacity(16 * 1024);
    let mut line = |text: &str| {
        html.push_str(text);
        html.push('\n');
    };
    line("<!doctype html>");
    line(r#"<html lang="fr">"#);
    line("<head>");
    line(r#"<meta charset="utf-8">"#);
    line(&format!(
        r#"<meta http-equiv="Content-Security-Policy" content="{CSP}">"#
    ));
    line(r#"<meta name="referrer" content="no-referrer">"#);
    line(r#"<meta name="viewport" content="width=device-width, initial-scale=1">"#);
    line(&format!(
        r#"<meta name="generator" content="{app} {version}">"#
    ));
    line(&format!(
        "<title>Synthèse du relevé {app} — {generated_local}</title>"
    ));
    line("<style>");
    line(STYLE.trim_end_matches('\n'));
    line("</style>");
    line("</head>");
    line("<body>");
    line("<header>");
    line(&format!("<h1>Synthèse du relevé {app}</h1>"));
    line(&format!(
        r#"<p class="sub">Exportée le {generated_local} (<code>{generated_iso}</code>) par {app} {version}. Relevé complet : les filtres d'affichage de {app} ne s'appliquent pas à cette page.</p>"#
    ));
    line(&context_block);
    line("</header>");
    line("");
    line(r#"<section class="fingerprint" aria-label="Matrice d'origine">"#);
    line(&format!(
        "<p><strong>Matrice d'origine :</strong> <code>{csv_name}</code> ({} octets), exportée en même temps que cette page.</p>",
        integer(export.csv_bytes)
    ));
    line(&format!(
        r#"<p><strong>Empreinte SHA-256 :</strong> <span class="hash">{sha}</span></p>"#
    ));
    line(&format!(
        r#"<p class="muted">Pour vérifier que cette page décrit bien ce fichier, calculer son empreinte et la comparer à celle-ci : <code>sha256sum {csv_name}</code> sous Linux, <code>Get-FileHash -Algorithm SHA256 {csv_name}</code> sous Windows (PowerShell). Si elles diffèrent, la page et le fichier ne correspondent pas.</p>"#
    ));
    line("</section>");
    line("");
    line("<h2>Indicateurs</h2>");
    line(r#"<div class="kpis">"#);
    line(&kpi_lines.join("\n"));
    line("</div>");
    line("");
    line("<h2>Interne / Internet</h2>");
    line(
        r#"<p class="muted">Un flux est « Internet » quand l'une de ses extrémités est une adresse IP publique.</p>"#,
    );
    line(&scope_table);
    line("");
    line("<h2>Chiffrement (indication)</h2>");
    line(&format!(
        r#"<p class="muted">Tirée du seul protocole reconnu par {app}, jamais du numéro de port : un flux dont le contenu n'est pas reconnu (TCP ou UDP seul) reste « Indéterminé ».</p>"#
    ));
    line(&confidentiality_table);
    line("");
    line("<h2>Usages (indication)</h2>");
    line(
        r#"<p class="muted">Tirés d'une table fixe qui range chaque protocole reconnu dans une catégorie ; « Non classé » quand le protocole ne dit rien de l'usage (une session TLS, par exemple).</p>"#,
    );
    line(&usage_table);
    line("");
    line("<h2>Répartition par protocole</h2>");
    line(&protocol_table);
    line("");
    line("<h2>Endpoints les plus actifs</h2>");
    line(&top_endpoints);
    line("");
    line("<h2>Flux les plus volumineux</h2>");
    line(&top_flows);
    line("");
    line(&format!("<h2>Endpoints ({})</h2>", count(endpoints.len())));
    line(&endpoint_table);
    line("");
    line(&format!("<h2>Flux ({})</h2>", count(flows.len())));
    line(&flow_table);
    line("");
    line(&format!(
        "<h2>Indications ({})</h2>",
        count(deductions.len())
    ));
    line(&format!(
        r#"<p class="muted">Tirées de tables fixes embarquées dans {app} (ports bien connus, préfixes de constructeurs, adresses notoires). Ce sont des indications, pas des conclusions : elles sont à confirmer par l'analyste.</p>"#
    ));
    line(&deduction_table);
    line("");
    line("<h2>Glossaire des termes présents</h2>");
    line(&glossary_section);
    line("");
    line(&format!(
        "<footer>Document produit hors ligne par {app} {version} à partir des seules données du relevé. Page statique : elle ne contient aucun script et ne charge aucune ressource.</footer>"
    ));
    line("</body>");
    line("</html>");
    html
}

/// Sens d'un flux : « ↔ » s'il est bidirectionnel, « → » sinon.
fn arrow(bidir: bool) -> &'static str {
    if bidir { "↔" } else { "→" }
}

#[cfg(test)]
mod tests {
    // La page doit rester inerte quel que soit le contenu du relevé, porter
    // l'empreinte du CSV joint et reprendre les chiffres de la vue.

    use super::*;
    use crate::investigation::dates::{FixedZone, parse_js_date};
    use crate::investigation::test_support::{edge, graph, node, ports, s};
    use lunar_flows_core::graph::{Edge, Node};

    /// Un label saisi par l'opérateur est une donnée libre : il ne doit
    /// jamais pouvoir devenir du balisage actif dans la page.
    const HOSTILE_LABEL: &str = r#"<script>alert("x")</script><img src=x onerror="alert(1)">"#;

    fn sample_graph() -> GraphData {
        graph(
            vec![
                (
                    "a",
                    Node {
                        id: s("a"),
                        name: s("plc-1"),
                        ip: s("192.168.1.10"),
                        mac: s("00:1d:9c:00:00:01"),
                        macs: vec![s("00:1d:9c:00:00:01")],
                        label: Some(s(HOSTILE_LABEL)),
                        ..node()
                    },
                ),
                (
                    "b",
                    Node {
                        id: s("b"),
                        name: s("scada"),
                        ip: s("192.168.1.20"),
                        mac: s("00:0c:29:00:00:02"),
                        macs: vec![s("00:0c:29:00:00:02")],
                        vlan_id: Some(10),
                        ..node()
                    },
                ),
                (
                    "c",
                    Node {
                        id: s("c"),
                        name: s("dns"),
                        ip: s("8.8.8.8"),
                        ..node()
                    },
                ),
            ],
            vec![
                (
                    "e1",
                    Edge {
                        id: s("e1"),
                        label: s("Modbus/TCP"),
                        destination_port: Some(502),
                        ports: ports(&[502]),
                        count: 120,
                        total_bytes: 9000,
                        bidir: true,
                        ..edge()
                    },
                ),
                (
                    "e2",
                    Edge {
                        id: s("e2"),
                        source: s("b"),
                        target: s("c"),
                        label: s("DNS"),
                        destination_port: Some(53),
                        ports: ports(&[53]),
                        count: 4,
                        total_bytes: 320,
                        ..edge()
                    },
                ),
            ],
        )
    }

    fn input() -> SynthesisExport {
        SynthesisExport {
            graph: sample_graph(),
            csv_file_name: s("20261012_DR_Synthese.csv"),
            csv_sha256: "a".repeat(64),
            csv_bytes: 4096,
            link_type: Some(s("ETHERNET")),
            site: Some(s(r#"Site <b>"Nord"</b>"#)),
            sensor: None,
            interface: Some(s("eth0")),
            app_name: s("LUNAR"),
            app_version: s("0.1.0"),
            period: SurveyPeriodView::Known {
                first: s("2026-10-12T08:30:00.000000Z"),
                last: s("2026-10-12T08:34:12.500000Z"),
            },
        }
    }

    fn render(export: &SynthesisExport) -> String {
        let generated_at = parse_js_date("2026-10-12T08:30:00Z").expect("date valide");
        render_synthesis_html(export, generated_at, &FixedZone::new(7200))
    }

    /// Vrai si une balise ouvrante de `html` satisfait `predicate` (nom et
    /// attributs en minuscules).
    fn any_tag(html: &str, predicate: impl Fn(&str) -> bool) -> bool {
        html.split('<')
            .skip(1)
            .filter_map(|chunk| chunk.split_once('>').map(|(tag, _)| tag.to_lowercase()))
            .any(|tag| predicate(&tag))
    }

    #[test]
    fn la_page_est_inerte_ni_script_ni_gestionnaire_ni_ressource_externe() {
        let html = render(&input());
        let lower = html.to_lowercase();
        assert!(!lower.contains("<script"), "balise script présente");
        assert!(
            !any_tag(&html, |tag| tag.split_whitespace().skip(1).any(
                |attribute| {
                    attribute.starts_with("on")
                        && attribute.split('=').next().is_some_and(|name| {
                            name.len() > 2 && name[2..].bytes().all(|b| b.is_ascii_lowercase())
                        })
                }
            )),
            "attribut de gestionnaire d'événement présent"
        );
        assert!(
            !any_tag(&html, |tag| {
                let name = tag.split_whitespace().next().unwrap_or("");
                [
                    "img", "iframe", "object", "embed", "link", "form", "base", "a",
                ]
                .contains(&name)
            }),
            "élément chargeant ou naviguant présent"
        );
        assert!(
            !any_tag(&html, |tag| ["src", "href", "action", "srcset"].iter().any(
                |attribute| {
                    tag.split(|c: char| c.is_whitespace())
                        .any(|word| word.starts_with(&format!("{attribute}=")))
                }
            )),
            "attribut de ressource présent"
        );
        assert!(
            !lower.contains("http://") && !lower.contains("https://"),
            "URL externe présente"
        );
        assert!(!lower.contains("url("), "ressource CSS présente");
    }

    #[test]
    fn la_politique_csp_interdit_tout_chargement() {
        let html = render(&input());
        assert!(
            html.contains(r#"<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'">"#),
            "CSP absente ou modifiée"
        );
    }

    #[test]
    fn les_donnees_du_releve_sont_echappees_jamais_interpretees() {
        let html = render(&input());
        assert!(
            html.contains(&escape_html(HOSTILE_LABEL)),
            "label hostile absent ou mal échappé"
        );
        assert!(html.contains("&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"));
        assert!(
            html.contains("<dd>Site &lt;b&gt;&quot;Nord&quot;&lt;/b&gt;</dd>"),
            "contexte mal échappé"
        );
        assert_eq!(
            escape_html(r#"a&b<c>"d"'e'"#),
            "a&amp;b&lt;c&gt;&quot;d&quot;&#39;e&#39;"
        );
    }

    #[test]
    fn la_page_porte_l_empreinte_et_le_nom_du_csv_joint() {
        let sha = "0123456789abcdef".repeat(4);
        let html = render(&SynthesisExport {
            csv_sha256: sha.clone(),
            ..input()
        });
        assert!(html.contains(&format!(r#"<span class="hash">{sha}</span>"#)));
        assert!(html.contains("<code>20261012_DR_Synthese.csv</code>"));
        assert!(html.contains("sha256sum 20261012_DR_Synthese.csv"));
        assert!(html.contains("Get-FileHash -Algorithm SHA256 20261012_DR_Synthese.csv"));
    }

    #[test]
    fn les_indicateurs_sont_ceux_de_la_vue_d_investigation() {
        let export = input();
        let html = render(&export);
        let stats = compute_stats(&GraphView::new(&export.graph));
        assert_eq!(stats.flows, 2);
        assert!(html.contains(&format!("<b>{}</b><span>Endpoints</span>", stats.endpoints)));
        assert!(html.contains(&format!("<b>{}</b><span>Flux</span>", stats.flows)));
        assert!(html.contains("<b>124</b><span>Paquets</span>"));
        assert!(html.contains("<h2>Flux (2)</h2>"));
        // Contexte : seuls les champs renseignés apparaissent.
        assert!(html.contains("<dt>Interface</dt><dd>eth0</dd>"));
        assert!(!html.contains("<dt>Capteur</dt>"));
        assert!(html.contains("<dt>Type de liaison</dt><dd>ETHERNET</dd>"));
        // Les déductions sont présentées comme des indications à confirmer.
        assert!(html.contains("pas des conclusions"));
    }

    #[test]
    fn meme_entree_meme_page() {
        assert_eq!(render(&input()), render(&input()));
    }

    #[test]
    fn un_releve_vide_donne_une_page_valide() {
        let html = render(&SynthesisExport {
            graph: GraphData::new(),
            link_type: None,
            site: None,
            interface: None,
            period: SurveyPeriodView::Empty,
            ..input()
        });
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<h2>Flux (0)</h2>"));
        assert!(html.contains("Aucun flux."));
        assert!(!html.contains(r#"<dl class="context">"#));
    }

    #[test]
    fn la_page_reprend_la_portee_le_chiffrement_et_les_usages() {
        let html = render(&input());
        assert!(html.contains("<h2>Interne / Internet</h2>"));
        assert!(html.contains("<h2>Chiffrement (indication)</h2>"));
        assert!(html.contains("<h2>Usages (indication)</h2>"));
        assert!(html.contains("jamais du numéro de port"));
        // Modbus/TCP et DNS sont reconnus en clair ; aucun flux chiffré.
        assert!(html.contains("<td>Industriel et opérationnel</td>"));
        assert!(html.contains("<td>Noms et découverte</td>"));
    }

    #[test]
    fn la_page_donne_la_duree_et_la_periode_du_releve() {
        let html = render(&input());
        assert!(html.contains("<b>4 min 12 s</b><span>Durée du relevé</span>"));
        assert!(html.contains("<dt>Période observée</dt>"));
        assert!(html.contains(
            "soit 4 min 12 s (UTC : 2026-10-12T08:30:00.000000Z à 2026-10-12T08:34:12.500000Z)"
        ));

        let unknown = render(&SynthesisExport {
            period: SurveyPeriodView::Unknown,
            ..input()
        });
        assert!(unknown.contains("<b>inconnue</b><span>Durée du relevé</span>"));
        assert!(unknown.contains("ne déclarait pas sa période"));

        let empty = render(&SynthesisExport {
            period: SurveyPeriodView::Empty,
            ..input()
        });
        assert!(!empty.contains("<dt>Période observée</dt>"));
    }

    #[test]
    fn l_en_tete_date_l_export_en_heure_locale_et_en_utc() {
        let html = render(&input());
        assert!(
            html.contains("<title>Synthèse du relevé LUNAR — 12 octobre 2026 à 10:30:00</title>")
        );
        assert!(html.contains("(<code>2026-10-12T08:30:00.000Z</code>)"));
        assert!(html.contains("du 12/10/2026 10:30:00 au 12/10/2026 10:34:12, soit 4 min 12 s"));
    }
}
