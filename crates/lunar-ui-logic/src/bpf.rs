//! Filtre BPF : construction et validation de l'expression à partir du
//! formulaire du panneau de filtre, presets rapides et état du formulaire
//! (aperçu éditable, badge « édition manuelle », « ↺ Sync auto »).
//!
//! L'état du formulaire est [`FilterForm`].

use std::fmt;

/// Direction d'une adresse ou d'un port : `src`, `dst` ou les deux.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Direction {
    /// Source ou destination (pas de préfixe dans l'expression).
    #[default]
    Any,
    /// Source uniquement (préfixe `src `).
    Src,
    /// Destination uniquement (préfixe `dst `).
    Dst,
}

impl Direction {
    /// Les trois directions, dans l'ordre de la liste déroulante.
    pub const ALL: [Direction; 3] = [Direction::Any, Direction::Src, Direction::Dst];

    /// Libellé de l'option dans la liste déroulante du panneau.
    pub const fn label(self) -> &'static str {
        match self {
            Direction::Any => "src ou dst",
            Direction::Src => "src",
            Direction::Dst => "dst",
        }
    }

    /// Préfixe BPF de la direction (`""`, `"src "` ou `"dst "`).
    const fn prefix(self) -> &'static str {
        match self {
            Direction::Any => "",
            Direction::Src => "src ",
            Direction::Dst => "dst ",
        }
    }
}

/// Options de couche (« Couche / Types »).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayerOptions {
    /// Trafic VLAN (802.1Q) : `vlan`.
    pub vlan: bool,
    /// Uniquement IPv4 : `ip`.
    pub only_ip4: bool,
    /// Exclure IPv6 : `not ip6`.
    pub exclude_ipv6: bool,
    /// Exclure ARP : `not arp`.
    pub exclude_arp: bool,
}

/// Protocoles cochés ; aucun coché = pas de restriction de protocole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProtoOptions {
    /// `tcp`.
    pub tcp: bool,
    /// `udp`.
    pub udp: bool,
    /// `icmp`.
    pub icmp: bool,
    /// `icmp6`.
    pub icmp6: bool,
}

/// Champs « Adresses IP » du formulaire (texte saisi tel quel).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IpFields {
    /// Hôte à inclure (`host …`).
    pub include_host: String,
    /// Hôte à exclure (`not host …`).
    pub exclude_host: String,
    /// Réseau à inclure, en CIDR (`net …`).
    pub include_net: String,
    /// Réseau à exclure, en CIDR (`not net …`).
    pub exclude_net: String,
    /// Direction appliquée aux quatre champs.
    pub direction: Direction,
}

/// Champs « Ports » du formulaire (texte saisi tel quel).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PortFields {
    /// Ports à inclure, séparés par des virgules.
    pub include: String,
    /// Ports à exclure, séparés par des virgules.
    pub exclude: String,
    /// Plage `début-fin`.
    pub range: String,
    /// Direction appliquée aux trois champs.
    pub direction: Direction,
}

/// Formulaire complet du panneau de filtre. [`Default`] donne le formulaire
/// vierge.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BpfFormState {
    /// Options de couche.
    pub opt: LayerOptions,
    /// Protocoles.
    pub proto: ProtoOptions,
    /// Adresses IP.
    pub ip: IpFields,
    /// Ports.
    pub ports: PortFields,
    /// Expression BPF brute, concaténée à la fin avec `and`.
    pub advanced_raw: String,
}

// --- Validation ---------------------------------------------------------

/// Erreur de saisie signalée sous un groupe de champs. [`fmt::Display`]
/// donne le message affiché.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldError {
    /// Hôte qui n'a pas la forme `a.b.c.d` (1 à 3 chiffres par octet).
    InvalidIp(String),
    /// Réseau qui n'a pas la forme `a.b.c.d/n` (1 ou 2 chiffres de masque).
    InvalidCidr(String),
    /// Liste de ports à inclure invalide.
    InvalidIncludePorts,
    /// Liste de ports à exclure invalide.
    InvalidExcludePorts,
    /// Plage de ports invalide.
    InvalidPortRange,
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldError::InvalidIp(value) => write!(f, "IP invalide : {value}"),
            FieldError::InvalidCidr(value) => write!(f, "CIDR invalide : {value}"),
            FieldError::InvalidIncludePorts => f.write_str("Ports à inclure invalides"),
            FieldError::InvalidExcludePorts => f.write_str("Ports à exclure invalides"),
            FieldError::InvalidPortRange => {
                f.write_str("Plage de ports invalide (ex: 10000-20000)")
            }
        }
    }
}

impl std::error::Error for FieldError {}

/// `true` si `s` n'est fait que de `min..=max` chiffres ASCII (`\d{min,max}`).
fn is_digits(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

/// Forme `^(\d{1,3}\.){3}\d{1,3}$` : quatre octets de 1 à 3 chiffres, sans
/// contrôle de valeur (`999.1.1.1` passe).
fn is_ip(s: &str) -> bool {
    let mut count = 0;
    for part in s.split('.') {
        count += 1;
        if count > 4 || !is_digits(part, 1, 3) {
            return false;
        }
    }
    count == 4
}

/// Forme `^(\d{1,3}\.){3}\d{1,3}\/\d{1,2}$`.
fn is_cidr(s: &str) -> bool {
    s.split_once('/')
        .is_some_and(|(addr, mask)| is_ip(addr) && is_digits(mask, 1, 2))
}

/// Un port : `^\s*\d{1,5}\s*$` et valeur ≤ 65535.
fn is_port(s: &str) -> bool {
    let s = s.trim();
    is_digits(s, 1, 5) && s.parse::<u32>().is_ok_and(|port| port <= 65535)
}

/// Liste de ports séparés par des virgules ; un élément vide la rend invalide.
fn is_port_list(s: &str) -> bool {
    s.split(',').all(is_port)
}

/// Plage `^\s*\d{1,5}\s*-\s*\d{1,5}\s*$`, bornes ≤ 65535 (sans exiger
/// début ≤ fin).
fn is_range(s: &str) -> bool {
    s.split_once('-')
        .is_some_and(|(start, end)| is_port(start) && is_port(end))
}

/// Erreurs des champs « Adresses IP » ; un champ vide n'est pas contrôlé.
pub fn validate_ip_fields(ip: &IpFields) -> Vec<FieldError> {
    let mut errors = Vec::new();
    for host in [&ip.include_host, &ip.exclude_host] {
        if !host.is_empty() && !is_ip(host) {
            errors.push(FieldError::InvalidIp(host.clone()));
        }
    }
    for net in [&ip.include_net, &ip.exclude_net] {
        if !net.is_empty() && !is_cidr(net) {
            errors.push(FieldError::InvalidCidr(net.clone()));
        }
    }
    errors
}

/// Erreurs des champs « Ports » ; un champ vide n'est pas contrôlé.
pub fn validate_port_fields(ports: &PortFields) -> Vec<FieldError> {
    let mut errors = Vec::new();
    if !ports.include.is_empty() && !is_port_list(&ports.include) {
        errors.push(FieldError::InvalidIncludePorts);
    }
    if !ports.exclude.is_empty() && !is_port_list(&ports.exclude) {
        errors.push(FieldError::InvalidExcludePorts);
    }
    if !ports.range.is_empty() && !is_range(&ports.range) {
        errors.push(FieldError::InvalidPortRange);
    }
    errors
}

// --- Construction ---------------------------------------------------------

/// Plusieurs clauses : `(a or b)` ; une seule : telle quelle ; aucune : `""`.
fn group_or(clauses: &[String]) -> String {
    match clauses {
        [] => String::new(),
        [single] => single.clone(),
        _ => format!("({})", clauses.join(" or ")),
    }
}

/// Assemble l'expression BPF (clauses jointes par `and`) depuis le
/// formulaire. Formulaire vierge : chaîne vide.
pub fn build_bpf_expression(state: &BpfFormState) -> String {
    let BpfFormState {
        opt,
        proto,
        ip,
        ports,
        advanced_raw,
    } = state;
    let mut clauses: Vec<String> = Vec::new();

    for (on, clause) in [
        (opt.vlan, "vlan"),
        (opt.only_ip4, "ip"),
        (opt.exclude_ipv6, "not ip6"),
        (opt.exclude_arp, "not arp"),
    ] {
        if on {
            clauses.push(clause.to_owned());
        }
    }

    let protos: Vec<String> = [
        (proto.tcp, "tcp"),
        (proto.udp, "udp"),
        (proto.icmp, "icmp"),
        (proto.icmp6, "icmp6"),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, name)| name.to_owned())
    .collect();
    if !protos.is_empty() {
        clauses.push(group_or(&protos));
    }

    let ip_dir = ip.direction.prefix();
    for (value, negate, kind) in [
        (&ip.include_host, false, "host"),
        (&ip.exclude_host, true, "host"),
        (&ip.include_net, false, "net"),
        (&ip.exclude_net, true, "net"),
    ] {
        if !value.is_empty() {
            let not = if negate { "not " } else { "" };
            clauses.push(format!("{not}{ip_dir}{kind} {value}"));
        }
    }

    let port_dir = ports.direction.prefix();
    for (list, negate) in [(&ports.include, false), (&ports.exclude, true)] {
        if list.trim().is_empty() {
            continue;
        }
        let parts: Vec<String> = list
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| format!("{port_dir}port {part}"))
            .collect();
        let not = if negate { "not " } else { "" };
        clauses.push(format!("{not}{}", group_or(&parts)));
    }
    let range = ports.range.trim();
    if !range.is_empty() {
        clauses.push(format!("{port_dir}portrange {range}"));
    }

    let raw = advanced_raw.trim();
    if !raw.is_empty() {
        clauses.push(raw.to_owned());
    }

    clauses.join(" and ").trim().to_owned()
}

// --- Presets ---------------------------------------------------------------

/// Presets rapides du panneau : chacun s'applique sur un formulaire vierge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BpfPreset {
    /// IPv4 uniquement.
    Ipv4,
    /// Web : TCP 80 et 443 en IPv4.
    Web,
    /// DNS : TCP ou UDP 53 en IPv4.
    Dns,
    /// NTP : UDP 123 en IPv4.
    Ntp,
    /// Segments TCP SYN seuls (ouvertures de connexion) en IPv4.
    Syn,
    /// Tout sauf ARP et IPv6.
    NoArpIpv6,
}

impl BpfPreset {
    /// Les presets, dans l'ordre des boutons du panneau.
    pub const ALL: [BpfPreset; 6] = [
        BpfPreset::Ipv4,
        BpfPreset::Web,
        BpfPreset::Dns,
        BpfPreset::Ntp,
        BpfPreset::Syn,
        BpfPreset::NoArpIpv6,
    ];

    /// Clé stable du preset.
    pub const fn key(self) -> &'static str {
        match self {
            BpfPreset::Ipv4 => "ipv4",
            BpfPreset::Web => "web",
            BpfPreset::Dns => "dns",
            BpfPreset::Ntp => "ntp",
            BpfPreset::Syn => "syn",
            BpfPreset::NoArpIpv6 => "no-arp-ipv6",
        }
    }

    /// Preset de clé `key`, ou `None`.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.key() == key)
    }

    /// Libellé du bouton.
    pub const fn label(self) -> &'static str {
        match self {
            BpfPreset::Ipv4 => "IPv4 uniquement",
            BpfPreset::Web => "Trafic web (80/443)",
            BpfPreset::Dns => "DNS (53)",
            BpfPreset::Ntp => "NTP (123)",
            BpfPreset::Syn => "SYN TCP uniquement",
            BpfPreset::NoArpIpv6 => "Tout sauf ARP/IPv6",
        }
    }

    /// Formulaire vierge auquel le preset est appliqué.
    pub fn form_state(self) -> BpfFormState {
        let mut state = BpfFormState::default();
        match self {
            BpfPreset::Ipv4 => state.opt.only_ip4 = true,
            BpfPreset::Web => {
                state.opt.only_ip4 = true;
                state.proto.tcp = true;
                state.ports.include = "80,443".to_owned();
            }
            BpfPreset::Dns => {
                state.opt.only_ip4 = true;
                state.proto.udp = true;
                state.proto.tcp = true;
                state.ports.include = "53".to_owned();
            }
            BpfPreset::Ntp => {
                state.opt.only_ip4 = true;
                state.proto.udp = true;
                state.ports.include = "123".to_owned();
            }
            BpfPreset::Syn => {
                state.opt.only_ip4 = true;
                state.proto.tcp = true;
                state.advanced_raw = "tcp[13] & 0x02 != 0 and tcp[13] & 0x10 = 0".to_owned();
            }
            BpfPreset::NoArpIpv6 => {
                state.opt.exclude_arp = true;
                state.opt.exclude_ipv6 = true;
            }
        }
        state
    }
}

// --- État du panneau de filtre ----------------------------------------------

/// Emplacement du filtre appliqué : actif tout de suite, ou en attente du
/// prochain démarrage quand une capture tourne déjà.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterSlot {
    /// Filtre actif (« Filtre actif »).
    Active,
    /// Filtre appliqué au prochain démarrage (« Prochain démarrage »).
    Pending,
}

impl FilterSlot {
    /// Emplacement d'un filtre appliqué selon que la capture tourne.
    pub const fn for_capture(running: bool) -> Self {
        if running {
            FilterSlot::Pending
        } else {
            FilterSlot::Active
        }
    }
}

/// État du formulaire de filtre : champs, aperçu généré ou édité à la main.
///
/// Tant que l'aperçu n'a pas été modifié à la main, il suit l'expression
/// générée par les champs. Une saisie dans l'aperçu le fige (badge
/// « édition manuelle ») jusqu'à [`FilterForm::sync_preview`]
/// (« ↺ Sync auto »), à une réinitialisation ou à un preset.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FilterForm {
    /// Champs du formulaire.
    form: BpfFormState,
    /// Texte de l'aperçu saisi à la main, `None` tant qu'il suit les champs.
    manual_preview: Option<String>,
}

impl FilterForm {
    /// Formulaire vierge, aperçu automatique (vide).
    pub fn new() -> Self {
        Self::default()
    }

    /// Champs du formulaire.
    pub fn form(&self) -> &BpfFormState {
        &self.form
    }

    /// Champs du formulaire, à modifier en place (cases, saisies). L'aperçu
    /// automatique suit ; un aperçu édité à la main reste tel quel.
    pub fn form_mut(&mut self) -> &mut BpfFormState {
        &mut self.form
    }

    /// Expression générée par les champs.
    pub fn auto_preview(&self) -> String {
        build_bpf_expression(&self.form)
    }

    /// Texte de l'aperçu : saisie manuelle, ou expression générée.
    pub fn preview_text(&self) -> String {
        match &self.manual_preview {
            Some(text) => text.clone(),
            None => self.auto_preview(),
        }
    }

    /// `true` si l'aperçu a été édité à la main : badge « édition
    /// manuelle » et bouton « ↺ Sync auto » visibles.
    pub fn is_manual_preview(&self) -> bool {
        self.manual_preview.is_some()
    }

    /// Saisie dans la zone d'aperçu : le texte devient l'aperçu, figé.
    pub fn edit_preview(&mut self, text: impl Into<String>) {
        self.manual_preview = Some(text.into());
    }

    /// « ↺ Sync auto » : l'aperçu reprend l'expression générée.
    pub fn sync_preview(&mut self) {
        self.manual_preview = None;
    }

    /// Formulaire vierge et aperçu automatique (« Réinitialiser », avant
    /// l'effacement du filtre côté moteur).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Applique un preset : formulaire vierge puis champs du preset.
    pub fn apply_preset(&mut self, preset: BpfPreset) {
        self.form = preset.form_state();
        self.manual_preview = None;
    }

    /// Erreurs des champs « Adresses IP ».
    pub fn ip_errors(&self) -> Vec<FieldError> {
        validate_ip_fields(&self.form.ip)
    }

    /// Erreurs des champs « Ports ».
    pub fn port_errors(&self) -> Vec<FieldError> {
        validate_port_fields(&self.form.ports)
    }

    /// Toutes les erreurs (affichées sous l'aperçu) : adresses puis ports.
    pub fn errors(&self) -> Vec<FieldError> {
        let mut errors = self.ip_errors();
        errors.extend(self.port_errors());
        errors
    }

    /// « Appliquer » actif : aperçu non vide et aucune erreur de champ.
    pub fn can_apply(&self) -> bool {
        !self.preview_text().trim().is_empty() && self.errors().is_empty()
    }

    /// Filtre à envoyer au moteur (aperçu sans espaces de bord), ou `None`
    /// si « Appliquer » est grisé.
    pub fn filter_to_apply(&self) -> Option<String> {
        self.can_apply()
            .then(|| self.preview_text().trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Formulaire vierge modifié par `patch`.
    fn state_with(patch: impl FnOnce(&mut BpfFormState)) -> BpfFormState {
        let mut state = BpfFormState::default();
        patch(&mut state);
        state
    }

    /// Champs IP complets (les quatre saisies et la direction).
    fn ip(include_host: &str, include_net: &str, direction: Direction) -> IpFields {
        IpFields {
            include_host: include_host.to_owned(),
            include_net: include_net.to_owned(),
            direction,
            ..IpFields::default()
        }
    }

    /// Champs ports complets.
    fn ports(include: &str, exclude: &str, range: &str) -> PortFields {
        PortFields {
            include: include.to_owned(),
            exclude: exclude.to_owned(),
            range: range.to_owned(),
            direction: Direction::Any,
        }
    }

    // --- Construction et validation ---

    #[test]
    fn formulaire_vide_donne_une_expression_vide() {
        assert_eq!(build_bpf_expression(&BpfFormState::default()), "");
    }

    #[test]
    fn options_de_couche_jointes_par_and() {
        let expr = build_bpf_expression(&state_with(|s| {
            s.opt = LayerOptions {
                vlan: true,
                only_ip4: true,
                exclude_ipv6: true,
                exclude_arp: true,
            };
        }));
        assert_eq!(expr, "vlan and ip and not ip6 and not arp");
    }

    #[test]
    fn protocoles_multiples_groupes_en_or() {
        let expr = build_bpf_expression(&state_with(|s| {
            s.proto.tcp = true;
            s.proto.udp = true;
        }));
        assert_eq!(expr, "(tcp or udp)");
    }

    #[test]
    fn un_seul_protocole_sans_parentheses() {
        let expr = build_bpf_expression(&state_with(|s| s.proto.icmp = true));
        assert_eq!(expr, "icmp");
    }

    #[test]
    fn hotes_et_reseaux_avec_direction() {
        let expr = build_bpf_expression(&state_with(|s| {
            s.ip = IpFields {
                include_host: "192.168.1.42".to_owned(),
                exclude_host: "10.0.0.1".to_owned(),
                include_net: "10.0.0.0/8".to_owned(),
                exclude_net: String::new(),
                direction: Direction::Src,
            };
        }));
        assert_eq!(
            expr,
            "src host 192.168.1.42 and not src host 10.0.0.1 and src net 10.0.0.0/8"
        );
    }

    #[test]
    fn listes_de_ports_incluses_exclues_et_plage() {
        let expr = build_bpf_expression(&state_with(|s| {
            s.ports = ports("80,443", "25", "10000-20000")
        }));
        assert_eq!(
            expr,
            "(port 80 or port 443) and not port 25 and portrange 10000-20000"
        );
    }

    #[test]
    fn expression_brute_concatenee_a_la_fin() {
        let expr = build_bpf_expression(&state_with(|s| {
            s.proto.tcp = true;
            s.advanced_raw = "tcp[13] & 0x02 != 0".to_owned();
        }));
        assert_eq!(expr, "tcp and tcp[13] & 0x02 != 0");
    }

    #[test]
    fn le_preset_web_produit_l_expression_attendue() {
        assert_eq!(
            build_bpf_expression(&BpfPreset::Web.form_state()),
            "ip and tcp and (port 80 or port 443)"
        );
    }

    #[test]
    fn tous_les_presets_produisent_une_expression_non_vide() {
        for preset in BpfPreset::ALL {
            let expr = build_bpf_expression(&preset.form_state());
            assert!(!expr.is_empty(), "preset {} vide", preset.key());
        }
    }

    #[test]
    fn ip_valides_ou_vides_aucune_erreur() {
        let fields = ip("192.168.1.1", "10.0.0.0/8", Direction::Any);
        assert_eq!(validate_ip_fields(&fields), vec![]);
    }

    #[test]
    fn ip_et_cidr_invalides_signales() {
        let errors = validate_ip_fields(&ip("192.168.1", "10.0.0.0", Direction::Any));
        assert_eq!(errors.len(), 2);
        assert!(errors[0].to_string().contains("192.168.1"));
        assert!(errors[1].to_string().contains("10.0.0.0"));
    }

    #[test]
    fn ports_et_plage_valides_aucune_erreur() {
        let fields = ports("80,443,22", "25", "10000-20000");
        assert_eq!(validate_port_fields(&fields), vec![]);
    }

    #[test]
    fn port_hors_bornes_et_plage_invalide_signales() {
        let errors = validate_port_fields(&ports("99999", "", "20000-99999"));
        assert_eq!(errors.len(), 2);
    }

    // --- Compléments : expressions des presets figées, validation fine ---

    #[test]
    fn expressions_des_presets_figees() {
        let expected = [
            (BpfPreset::Ipv4, "ip"),
            (BpfPreset::Web, "ip and tcp and (port 80 or port 443)"),
            (BpfPreset::Dns, "ip and (tcp or udp) and port 53"),
            (BpfPreset::Ntp, "ip and udp and port 123"),
            (
                BpfPreset::Syn,
                "ip and tcp and tcp[13] & 0x02 != 0 and tcp[13] & 0x10 = 0",
            ),
            (BpfPreset::NoArpIpv6, "not ip6 and not arp"),
        ];
        for (preset, expr) in expected {
            assert_eq!(build_bpf_expression(&preset.form_state()), expr);
        }
    }

    #[test]
    fn presets_retrouves_par_cle_et_libelles_francais() {
        for preset in BpfPreset::ALL {
            assert_eq!(BpfPreset::from_key(preset.key()), Some(preset));
            assert!(!preset.label().contains("only"), "{}", preset.label());
        }
        assert_eq!(BpfPreset::from_key("inconnu"), None);
    }

    #[test]
    fn messages_d_erreur_attendus() {
        let errors = validate_port_fields(&ports("a", "b", "c"));
        let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
        assert_eq!(
            messages,
            [
                "Ports à inclure invalides",
                "Ports à exclure invalides",
                "Plage de ports invalide (ex: 10000-20000)",
            ]
        );
        assert_eq!(
            FieldError::InvalidIp("x".to_owned()).to_string(),
            "IP invalide : x"
        );
        assert_eq!(
            FieldError::InvalidCidr("y".to_owned()).to_string(),
            "CIDR invalide : y"
        );
    }

    #[test]
    fn formes_d_adresse_comme_les_regex_d_origine() {
        assert!(is_ip("999.1.1.1"), "pas de contrôle de valeur");
        assert!(!is_ip("1.1.1.1.1"));
        assert!(!is_ip("1.1.1."));
        assert!(!is_ip(" 1.1.1.1"));
        assert!(!is_ip("1234.1.1.1"));
        assert!(is_cidr("10.0.0.0/8"));
        assert!(!is_cidr("10.0.0.0/123"));
        assert!(!is_cidr("10.0.0.0/8/8"));
        assert!(!is_cidr("10.0.0.0/"));
    }

    #[test]
    fn formes_de_ports_comme_les_regex_d_origine() {
        assert!(is_port_list(" 80 , 443 "));
        assert!(is_port_list("65535"));
        assert!(!is_port_list("65536"));
        assert!(!is_port_list("80,,443"), "élément vide");
        assert!(!is_port_list("123456"));
        assert!(is_range(" 10 - 20 "));
        assert!(is_range("20-10"), "début > fin accepté");
        assert!(!is_range("10-20-30"));
        assert!(!is_range("10"));
    }

    #[test]
    fn ports_vides_dans_la_liste_ignores_a_la_construction() {
        let expr = build_bpf_expression(&state_with(|s| {
            s.ports = ports(" 80, ,443 ", "", " 1-2 ");
            s.ports.direction = Direction::Dst;
        }));
        assert_eq!(expr, "(dst port 80 or dst port 443) and dst portrange 1-2");
    }

    // --- Panneau : aperçu, édition manuelle, Sync auto ---

    #[test]
    fn apercu_suit_les_champs_tant_qu_il_n_est_pas_edite() {
        let mut form = FilterForm::new();
        assert_eq!(form.preview_text(), "");
        assert!(!form.is_manual_preview());
        assert!(!form.can_apply(), "aperçu vide");
        form.form_mut().proto.udp = true;
        assert_eq!(form.preview_text(), "udp");
        assert_eq!(form.filter_to_apply().as_deref(), Some("udp"));
    }

    #[test]
    fn edition_manuelle_fige_l_apercu_jusqu_a_sync_auto() {
        let mut form = FilterForm::new();
        form.form_mut().proto.tcp = true;
        form.edit_preview("  udp and port 53 ");
        assert!(form.is_manual_preview());
        form.form_mut().proto.icmp = true;
        assert_eq!(form.preview_text(), "  udp and port 53 ");
        assert_eq!(form.filter_to_apply().as_deref(), Some("udp and port 53"));
        form.sync_preview();
        assert!(!form.is_manual_preview());
        assert_eq!(form.preview_text(), "(tcp or icmp)");
    }

    #[test]
    fn preset_et_reinitialisation_repartent_d_un_formulaire_vierge() {
        let mut form = FilterForm::new();
        form.form_mut().ip.include_host = "10.0.0.1".to_owned();
        form.edit_preview("arp");
        form.apply_preset(BpfPreset::Ntp);
        assert!(!form.is_manual_preview());
        assert_eq!(
            form.form().ip,
            IpFields::default(),
            "preset sur formulaire vierge"
        );
        assert_eq!(form.preview_text(), "ip and udp and port 123");
        form.reset();
        assert_eq!(form, FilterForm::new());
    }

    #[test]
    fn erreurs_de_champ_bloquent_appliquer_meme_en_edition_manuelle() {
        let mut form = FilterForm::new();
        form.form_mut().ip.include_host = "192.168.1".to_owned();
        form.form_mut().ports.range = "1-99999".to_owned();
        let errors = form.errors();
        assert_eq!(
            errors,
            [
                FieldError::InvalidIp("192.168.1".to_owned()),
                FieldError::InvalidPortRange,
            ]
        );
        form.edit_preview("tcp");
        assert!(!form.can_apply());
        assert_eq!(form.filter_to_apply(), None);
    }

    #[test]
    fn filtre_applique_pendant_une_capture_attend_le_prochain_demarrage() {
        assert_eq!(FilterSlot::for_capture(true), FilterSlot::Pending);
        assert_eq!(FilterSlot::for_capture(false), FilterSlot::Active);
    }

    #[test]
    fn libelles_de_direction() {
        let labels: Vec<_> = Direction::ALL.iter().map(|d| d.label()).collect();
        assert_eq!(labels, ["src ou dst", "src", "dst"]);
    }
}
