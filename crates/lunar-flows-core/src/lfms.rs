//! Préambule LFMS : métadonnées du relevé portées par la première ligne des
//! exports CSV.
//!
//! Un relevé = un réseau = un type de liaison : plutôt qu'une colonne DLT
//! répétée à chaque ligne, l'export écrit une ligne `#LFMS key=value …` avant
//! la ligne d'en-têtes. Un export antérieur à ce préambule s'importe avec un
//! DLT implicite Ethernet. Le préambule ne porte volontairement **pas** de
//! date d'export : deux exports de la même matrice doivent rester identiques
//! octet pour octet (déterminisme). Il porte en revanche, quand LUNAR
//! l'a observée, la période du relevé (`first_seen=` / `last_seen=`) :
//! tirée des horodatages des paquets, elle ne dépend que des
//! données.

use std::io::BufRead;
use std::path::Path;
use std::time::SystemTime;

use chrono::{DateTime, SecondsFormat, Utc};

use packet_parser::LinkType;

use crate::{LunarCoreError, Result};

/// Version du format de préambule écrite par cette version de LUNAR. La
/// version 2 ajoute le contexte de relevé ([`SurveyContext`]) ; la lecture
/// des fichiers v1 est inchangée.
pub const LFMS_VERSION: &str = "2";

/// Marqueur de la ligne de préambule, en tête de fichier.
pub const PREAMBLE_MARKER: &str = "#LFMS";

/// Contexte d'un relevé : où et par quoi il a été capté.
///
/// Ces informations n'existent pas dans le paquet : elles sont déclarées par
/// l'opérateur au moment de l'arrêt de la capture ou de l'enregistrement
/// (à la manière de Wireshark). Elles décrivent le
/// **relevé entier** et restent volontairement hors de la clé d'identité des
/// flux et des nœuds : le même paquet peut être vu par plusieurs capteurs, et
/// mettre le capteur dans la clé empêcherait de reconnaître ce recouvrement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SurveyContext {
    /// Site d'audit (« Usine Nord », « Poste 63 kV »…).
    pub site: Option<String>,
    /// Capteur ayant réalisé le relevé (nom de la sonde ou de la machine).
    pub sensor: Option<String>,
    /// Interface d'écoute (`eth0`…), connue en capture live.
    pub interface: Option<String>,
}

impl SurveyContext {
    /// Aucun champ renseigné : le préambule n'émet alors aucune clé de
    /// contexte (un export sans contexte reste identique à un export v1 à la
    /// version près).
    pub fn is_empty(&self) -> bool {
        self.site.is_none() && self.sensor.is_none() && self.interface.is_none()
    }

    /// Normalise une saisie utilisateur : une valeur vide ou uniquement
    /// blanche vaut « non renseigné », les blancs de bord sont retirés.
    pub fn normalized_field(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }
}

/// Période observée d'un relevé : horodatages du premier et du dernier
/// paquet vus (capture ou import PCAP), bornes incluses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedPeriod {
    /// Horodatage du premier paquet observé (borne incluse).
    pub first: SystemTime,
    /// Horodatage du dernier paquet observé (borne incluse).
    pub last: SystemTime,
}

impl ObservedPeriod {
    /// Période réduite à un instant.
    pub fn at(timestamp: SystemTime) -> Self {
        Self {
            first: timestamp,
            last: timestamp,
        }
    }

    /// Étend la période pour inclure `timestamp` (paquets hors ordre compris).
    pub fn include(&mut self, timestamp: SystemTime) {
        if timestamp < self.first {
            self.first = timestamp;
        }
        if timestamp > self.last {
            self.last = timestamp;
        }
    }

    /// Plus petite période couvrant les deux.
    pub fn union(self, other: Self) -> Self {
        Self {
            first: self.first.min(other.first),
            last: self.last.max(other.last),
        }
    }
}

/// Ce que l'on sait de la période d'un relevé.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SurveyPeriod {
    /// Rien d'observé : matrice vierge.
    #[default]
    Empty,
    /// Premier et dernier paquet connus.
    Known(ObservedPeriod),
    /// Une partie du relevé vient d'une matrice qui ne déclare pas sa
    /// période (export sans période, classeur retravaillé) : la durée
    /// réelle ne peut pas être établie, et on ne la devine pas.
    Unknown,
}

impl SurveyPeriod {
    /// Prend en compte un paquet observé. Une période inconnue le reste :
    /// elle couvre des données dont on ignore les dates.
    pub fn observe(&mut self, timestamp: SystemTime) {
        match self {
            Self::Empty => *self = Self::Known(ObservedPeriod::at(timestamp)),
            Self::Known(period) => period.include(timestamp),
            Self::Unknown => {}
        }
    }

    /// Réunit la période d'une autre source (fusion de relevés).
    pub fn merge(&mut self, other: Self) {
        *self = match (*self, other) {
            (Self::Empty, other) => other,
            (current, Self::Empty) => current,
            (Self::Known(a), Self::Known(b)) => Self::Known(a.union(b)),
            _ => Self::Unknown,
        };
    }

    /// La période, si elle est connue.
    pub fn known(&self) -> Option<ObservedPeriod> {
        match self {
            Self::Known(period) => Some(*period),
            _ => None,
        }
    }

    /// Période apportée par un fichier importé : celle de son préambule, ou
    /// inconnue s'il porte des lignes sans la déclarer.
    pub fn of_import(declared: Option<ObservedPeriod>, has_rows: bool) -> Self {
        match (declared, has_rows) {
            (Some(period), _) => Self::Known(period),
            (None, true) => Self::Unknown,
            (None, false) => Self::Empty,
        }
    }
}

/// Horodatage du préambule : RFC 3339 en UTC à la microseconde, sans espace.
pub fn format_timestamp(timestamp: SystemTime) -> String {
    DateTime::<Utc>::from(timestamp).to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// Inverse de [`format_timestamp`] : lit une date RFC 3339 (tout décalage
/// accepté) et la ramène en `SystemTime`. `key` ne sert qu'au message
/// d'erreur, qui cite le champ illisible du préambule.
fn parse_timestamp(key: &str, value: &str) -> std::result::Result<SystemTime, String> {
    DateTime::parse_from_rfc3339(value)
        .map(SystemTime::from)
        .map_err(|_| format!("date illisible dans le préambule : « {key}={value} »"))
}

/// Métadonnées lues dans une ligne de préambule `#LFMS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfmsPreamble {
    /// Version du format annoncée par le fichier.
    pub version: String,
    /// Type de liaison du relevé (`dlt=`), absent si le fichier ne le
    /// déclare pas.
    pub link_type: Option<LinkType>,
    /// Contexte du relevé (v2). Vide pour un fichier v1.
    pub context: SurveyContext,
    /// Période observée (`first_seen=` / `last_seen=`), absente d'un fichier
    /// qui ne la déclare pas ou d'une matrice sans paquet daté.
    pub period: Option<ObservedPeriod>,
    /// Filtre d'un export partiel (`filter=`) : le fichier ne porte que les
    /// flux retenus par ce filtre, décrit en clair. Absent d'un export
    /// complet.
    pub filter: Option<String>,
}

/// Encode une valeur libre pour tenir dans un champ `clé=valeur` séparé par
/// des espaces : tout ce qui n'est pas alphanumérique ASCII ou `-._~` passe
/// en `%XX` (UTF-8 octet par octet). Un nom de site avec espaces, accents ou
/// `=` traverse ainsi le préambule sans le rendre ambigu.
fn encode_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char)
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

/// Inverse de [`encode_value`]. Une séquence `%` invalide est une erreur :
/// une métadonnée illisible ne doit jamais être devinée.
fn decode_value(value: &str) -> std::result::Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = value
                .get(index + 1..index + 3)
                .ok_or_else(|| format!("échappement incomplet dans « {value} »"))?;
            decoded.push(
                u8::from_str_radix(hex, 16)
                    .map_err(|_| format!("échappement invalide « %{hex} » dans « {value} »"))?,
            );
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| format!("valeur non UTF-8 dans « {value} »"))
}

/// Nom canonique d'un type de liaison pour le préambule et les messages
/// d'erreur : mnémonique `LINKTYPE_*` pour les liaisons décodées par
/// `packet_parser`, valeur numérique sinon — jamais de nom inventé.
pub fn link_type_name(link_type: LinkType) -> String {
    match link_type {
        LinkType::ETHERNET => "ETHERNET".to_string(),
        LinkType::NULL => "NULL".to_string(),
        LinkType::RAW => "RAW".to_string(),
        LinkType::IEEE802_11 => "IEEE802_11".to_string(),
        LinkType::LINUX_SLL => "LINUX_SLL".to_string(),
        LinkType::LINUX_SLL2 => "LINUX_SLL2".to_string(),
        other => other.0.to_string(),
    }
}

/// Inverse exact de [`link_type_name`] : mnémonique connu ou valeur
/// numérique. `None` si le texte n'est ni l'un ni l'autre.
pub fn link_type_from_text(text: &str) -> Option<LinkType> {
    match text {
        "ETHERNET" => Some(LinkType::ETHERNET),
        "NULL" => Some(LinkType::NULL),
        "RAW" => Some(LinkType::RAW),
        "IEEE802_11" => Some(LinkType::IEEE802_11),
        "LINUX_SLL" => Some(LinkType::LINUX_SLL),
        "LINUX_SLL2" => Some(LinkType::LINUX_SLL2),
        other => other.parse::<u32>().ok().map(LinkType),
    }
}

/// Ligne de préambule écrite en tête d'un export de matrice, sans contexte
/// de relevé (capture non qualifiée, exports internes).
pub fn format_preamble(link_type: Option<LinkType>) -> String {
    format_preamble_with_context(link_type, &SurveyContext::default())
}

/// Préambule complet : type de liaison, contexte et période observée.
pub fn format_preamble_with_period(
    link_type: Option<LinkType>,
    context: &SurveyContext,
    period: Option<ObservedPeriod>,
) -> String {
    format_preamble_with_filter(link_type, context, period, None)
}

/// Préambule d'un export éventuellement partiel : type de liaison, contexte,
/// période observée et, si l'export ne porte qu'une partie des flux, le
/// filtre qui les a retenus (`filter=`, percent-encodé). Un filtre vide
/// n'émet aucune clé : l'export complet reste la même ligne qu'avant.
pub fn format_preamble_with_filter(
    link_type: Option<LinkType>,
    context: &SurveyContext,
    period: Option<ObservedPeriod>,
    filter: Option<&str>,
) -> String {
    let mut line = format_preamble_with_context(link_type, context);
    if let Some(period) = period {
        line.push_str(&format!(
            " first_seen={} last_seen={}",
            format_timestamp(period.first),
            format_timestamp(period.last)
        ));
    }
    if let Some(filter) = filter.map(str::trim).filter(|filter| !filter.is_empty()) {
        line.push_str(&format!(" filter={}", encode_value(filter)));
    }
    line
}

/// Ligne de préambule portant le contexte du relevé. Les champs non
/// renseignés n'émettent aucune clé : un export sans contexte reste
/// exactement l'ancienne ligne, à la version près.
pub fn format_preamble_with_context(
    link_type: Option<LinkType>,
    context: &SurveyContext,
) -> String {
    let mut line = format!("{PREAMBLE_MARKER} version={LFMS_VERSION}");
    if let Some(link_type) = link_type {
        line.push_str(&format!(" dlt={}", link_type_name(link_type)));
    }
    for (key, value) in [
        ("site", context.site.as_ref()),
        ("sensor", context.sensor.as_ref()),
        ("interface", context.interface.as_ref()),
    ] {
        if let Some(value) = value {
            line.push_str(&format!(" {key}={}", encode_value(value)));
        }
    }
    line
}

/// Parse une ligne de préambule. `Ok(None)` si la ligne n'en est pas une
/// (fichier d'un export antérieur) ; erreur si le préambule est présent mais
/// illisible — une métadonnée qu'on ne sait pas lire ne doit pas être
/// silencieusement ignorée. Les clés inconnues sont tolérées (extensions
/// futures du format).
pub fn parse_preamble(line: &str) -> std::result::Result<Option<LfmsPreamble>, String> {
    let Some(fields) = line.trim_end().strip_prefix(PREAMBLE_MARKER) else {
        return Ok(None);
    };

    let mut version: Option<String> = None;
    let mut link_type: Option<LinkType> = None;
    let mut context = SurveyContext::default();
    let mut first_seen: Option<SystemTime> = None;
    let mut last_seen: Option<SystemTime> = None;
    let mut filter: Option<String> = None;
    for field in fields.split_whitespace() {
        let Some((key, value)) = field.split_once('=') else {
            return Err(format!("champ de préambule invalide : « {field} »"));
        };
        match key {
            "version" => version = Some(value.to_string()),
            "dlt" => {
                link_type = Some(link_type_from_text(value).ok_or_else(|| {
                    format!("type de liaison illisible dans le préambule : « {value} »")
                })?);
            }
            "site" => context.site = Some(decode_value(value)?),
            "sensor" => context.sensor = Some(decode_value(value)?),
            "interface" => context.interface = Some(decode_value(value)?),
            "first_seen" => first_seen = Some(parse_timestamp(key, value)?),
            "last_seen" => last_seen = Some(parse_timestamp(key, value)?),
            // Une clé `filter=` vide ne déclare rien : seul un filtre décrit
            // marque le fichier comme partiel.
            "filter" => {
                filter = Some(decode_value(value)?).filter(|text| !text.trim().is_empty());
            }
            // Clés inconnues tolérées : un fichier plus récent reste lisible.
            _ => {}
        }
    }

    // Une borne sans l'autre, ou des bornes inversées : préambule incohérent,
    // refusé plutôt que corrigé en silence.
    let period = match (first_seen, last_seen) {
        (None, None) => None,
        (Some(first), Some(last)) if first <= last => Some(ObservedPeriod { first, last }),
        (Some(_), Some(_)) => {
            return Err(
                "période du préambule incohérente : first_seen après last_seen".to_string(),
            );
        }
        _ => {
            return Err(
                "période du préambule incomplète : first_seen et last_seen vont ensemble"
                    .to_string(),
            );
        }
    };

    Ok(Some(LfmsPreamble {
        version: version.unwrap_or_else(|| LFMS_VERSION.to_string()),
        link_type,
        context,
        period,
        filter,
    }))
}

/// Lit le préambule d'un fichier de matrice CSV (sa première ligne).
/// `Ok(None)` pour un export antérieur au préambule.
pub fn read_preamble(path: &Path) -> Result<Option<LfmsPreamble>> {
    let file = std::fs::File::open(path).map_err(|e| LunarCoreError::InvalidCsv {
        path: path.to_path_buf(),
        message: format!("ouverture impossible: {e}"),
    })?;
    let mut first_line = String::new();
    std::io::BufReader::new(file)
        .read_line(&mut first_line)
        .map_err(|e| LunarCoreError::InvalidCsv {
            path: path.to_path_buf(),
            message: format!("lecture impossible: {e}"),
        })?;
    parse_preamble(&first_line).map_err(|message| LunarCoreError::InvalidCsv {
        path: path.to_path_buf(),
        message,
    })
}

/// Type de liaison déclaré par un fichier de matrice : celui du préambule,
/// sinon Ethernet implicite (exports antérieurs au préambule).
pub fn matrix_file_link_type(path: &Path) -> Result<LinkType> {
    Ok(read_preamble(path)?
        .and_then(|preamble| preamble.link_type)
        .unwrap_or(LinkType::ETHERNET))
}

/// Refuse un relevé dont le DLT n'a pas de projection d'identité LFMS définie
/// dans cette version, plutôt que de le reconstruire sous un autre type de
/// liaison. Garde **commune** à tous les formats de matrice (CSV, XLSX) : deux
/// implémentations finiraient par diverger sur ce que LUNAR accepte de relire.
pub fn ensure_reimportable(path: &Path, link_type: LinkType) -> Result<()> {
    if matches!(
        link_type,
        LinkType::ETHERNET
            | LinkType::NULL
            | LinkType::RAW
            | LinkType::LINUX_SLL
            | LinkType::LINUX_SLL2
    ) {
        return Ok(());
    }
    Err(LunarCoreError::UnreimportableLinkType {
        path: path.to_path_buf(),
        label: link_type_name(link_type),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn at(secs: u64, micros: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs) + Duration::from_micros(micros)
    }

    /// Le filtre d'un export partiel traverse le préambule tel quel, espaces,
    /// accents et `=` compris, et un export complet reste l'ancienne ligne.
    #[test]
    fn filter_round_trips_through_the_preamble_and_is_absent_from_a_full_export() {
        let filter = "protocoles : TLS, DNS ; recherche : « 10.0.0 = prod »";
        let line = format_preamble_with_filter(
            Some(LinkType::ETHERNET),
            &SurveyContext::default(),
            None,
            Some(filter),
        );
        assert!(
            line.starts_with("#LFMS version=2 dlt=ETHERNET filter=protocoles%20%3A%20TLS"),
            "{line}"
        );
        assert_eq!(line.split_whitespace().count(), 4, "une seule clé de plus");
        let parsed = parse_preamble(&line).expect("lisible").expect("préambule");
        assert_eq!(parsed.filter.as_deref(), Some(filter));

        for absent in [None, Some(""), Some("   ")] {
            assert_eq!(
                format_preamble_with_filter(
                    Some(LinkType::ETHERNET),
                    &SurveyContext::default(),
                    None,
                    absent,
                ),
                format_preamble(Some(LinkType::ETHERNET)),
                "un filtre absent ou vide n'émet aucune clé"
            );
        }
        let full = parse_preamble("#LFMS version=2 dlt=ETHERNET")
            .expect("lisible")
            .expect("préambule");
        assert_eq!(full.filter, None);
        let empty = parse_preamble("#LFMS version=2 dlt=ETHERNET filter=")
            .expect("lisible")
            .expect("préambule");
        assert_eq!(empty.filter, None, "une clé vide ne déclare rien");
        assert!(
            parse_preamble("#LFMS version=2 filter=%ZZ").is_err(),
            "un filtre illisible n'est pas deviné"
        );
    }

    #[test]
    fn period_round_trips_through_the_preamble_to_the_microsecond() {
        let period = ObservedPeriod {
            first: at(1_760_000_000, 123_456),
            last: at(1_760_000_252, 7),
        };
        let line = format_preamble_with_period(
            Some(LinkType::ETHERNET),
            &SurveyContext::default(),
            Some(period),
        );
        assert_eq!(
            line,
            "#LFMS version=2 dlt=ETHERNET first_seen=2025-10-09T08:53:20.123456Z \
             last_seen=2025-10-09T08:57:32.000007Z"
        );
        let parsed = parse_preamble(&line).expect("lisible").expect("préambule");
        assert_eq!(parsed.period, Some(period));
        // Sans période : l'ancienne ligne, à l'identique.
        assert_eq!(
            format_preamble_with_period(Some(LinkType::ETHERNET), &SurveyContext::default(), None),
            format_preamble(Some(LinkType::ETHERNET))
        );
    }

    #[test]
    fn an_incoherent_period_is_refused_not_guessed() {
        assert!(parse_preamble("#LFMS version=2 first_seen=2025-10-09T08:53:20Z").is_err());
        assert!(parse_preamble("#LFMS version=2 last_seen=2025-10-09T08:53:20Z").is_err());
        assert!(
            parse_preamble(
                "#LFMS version=2 first_seen=2025-10-09T09:00:00Z last_seen=2025-10-09T08:00:00Z"
            )
            .is_err()
        );
        assert!(
            parse_preamble("#LFMS version=2 first_seen=hier last_seen=2025-10-09T08:00:00Z")
                .is_err()
        );
    }

    #[test]
    fn period_grows_with_packets_and_merges_with_sources() {
        let mut period = SurveyPeriod::Empty;
        assert_eq!(period.known(), None);
        period.observe(at(100, 0));
        period.observe(at(40, 0)); // paquet hors ordre
        period.observe(at(70, 0));
        assert_eq!(
            period,
            SurveyPeriod::Known(ObservedPeriod {
                first: at(40, 0),
                last: at(100, 0)
            })
        );

        period.merge(SurveyPeriod::Known(ObservedPeriod::at(at(500, 0))));
        assert_eq!(
            period.known().map(|p| (p.first, p.last)),
            Some((at(40, 0), at(500, 0)))
        );
        period.merge(SurveyPeriod::Empty);
        assert!(period.known().is_some(), "un fichier vide ne change rien");

        // Une source sans période rend la durée inconnue, et elle le reste.
        period.merge(SurveyPeriod::of_import(None, true));
        assert_eq!(period, SurveyPeriod::Unknown);
        period.observe(at(900, 0));
        assert_eq!(period, SurveyPeriod::Unknown);

        assert_eq!(SurveyPeriod::of_import(None, false), SurveyPeriod::Empty);
    }

    #[test]
    fn preamble_round_trips_every_supported_link_type() {
        for link_type in [
            LinkType::ETHERNET,
            LinkType::NULL,
            LinkType::RAW,
            LinkType::IEEE802_11,
            LinkType::LINUX_SLL,
            LinkType::LINUX_SLL2,
            LinkType(147), // DLT_USER0 : pas de mnémonique, valeur numérique
        ] {
            let line = format_preamble(Some(link_type));
            let parsed = parse_preamble(&line)
                .expect("préambule écrit par nous")
                .expect("ligne de préambule");
            assert_eq!(parsed.link_type, Some(link_type), "aller-retour {line}");
            assert_eq!(parsed.version, LFMS_VERSION);
        }
    }

    #[test]
    fn a_regular_header_line_is_not_a_preamble() {
        assert_eq!(parse_preamble("mac_source,mac_destination,…"), Ok(None));
    }

    #[test]
    fn unknown_keys_are_tolerated_but_bad_dlt_is_an_error() {
        let parsed = parse_preamble("#LFMS version=1 dlt=ETHERNET futur=2026-07-14")
            .expect("clé inconnue tolérée")
            .expect("préambule");
        assert_eq!(parsed.link_type, Some(LinkType::ETHERNET));

        assert!(parse_preamble("#LFMS version=1 dlt=N_IMPORTE_QUOI").is_err());
        assert!(parse_preamble("#LFMS champ_sans_valeur").is_err());
    }

    /// Le contexte de relevé traverse le préambule sans perte, y compris des
    /// valeurs libres contenant espaces, accents, `=` et `%` — un nom de site
    /// est saisi par un humain, pas contraint.
    #[test]
    fn survey_context_round_trips_free_text_values() {
        let context = SurveyContext {
            site: Some("Usine Nord — atelier 3".to_string()),
            sensor: Some("sonde=A 100%".to_string()),
            interface: Some("eth0".to_string()),
        };

        let line = format_preamble_with_context(Some(LinkType::ETHERNET), &context);
        assert!(
            !line.contains("Usine Nord"),
            "les espaces doivent être échappés : {line}"
        );

        let parsed = parse_preamble(&line)
            .expect("préambule écrit par nous")
            .expect("ligne de préambule");
        assert_eq!(parsed.context, context, "aller-retour exact");
        assert_eq!(parsed.link_type, Some(LinkType::ETHERNET));
        assert_eq!(parsed.version, "2");
    }

    /// Sans contexte, la ligne reste celle d'avant (à la version près) : un
    /// export non qualifié n'invente aucune métadonnée.
    #[test]
    fn a_preamble_without_context_emits_no_context_key() {
        let line = format_preamble(Some(LinkType::ETHERNET));
        assert_eq!(line, "#LFMS version=2 dlt=ETHERNET");

        let parsed = parse_preamble(&line).expect("valide").expect("préambule");
        assert!(parsed.context.is_empty());
    }

    /// Un relevé v1 se lit inchangé, avec un contexte simplement vide.
    #[test]
    fn a_v1_file_reads_with_an_empty_context() {
        let parsed = parse_preamble("#LFMS version=1 dlt=LINUX_SLL")
            .expect("valide")
            .expect("préambule");
        assert_eq!(parsed.version, "1");
        assert_eq!(parsed.link_type, Some(LinkType::LINUX_SLL));
        assert!(parsed.context.is_empty(), "aucun contexte inventé");
    }

    /// Un échappement corrompu est refusé plutôt que deviné.
    #[test]
    fn a_broken_escape_is_refused() {
        assert!(parse_preamble("#LFMS version=2 site=Usine%").is_err());
        assert!(parse_preamble("#LFMS version=2 site=Usine%ZZ").is_err());
    }

    /// Les saisies vides ou blanches valent « non renseigné » : le préambule
    /// ne se remplit pas de champs vides.
    #[test]
    fn blank_user_input_is_normalized_to_absent() {
        assert_eq!(SurveyContext::normalized_field(Some("   ")), None);
        assert_eq!(SurveyContext::normalized_field(Some("")), None);
        assert_eq!(SurveyContext::normalized_field(None), None);
        assert_eq!(
            SurveyContext::normalized_field(Some("  Poste 63 kV  ")),
            Some("Poste 63 kV".to_string())
        );
    }
}
