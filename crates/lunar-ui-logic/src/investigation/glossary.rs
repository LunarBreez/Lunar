//! Glossaire embarqué (hors ligne) : définitions brèves des protocoles et
//! termes techniques susceptibles d'apparaître dans un relevé. LUNAR étant
//! prévu pour des réseaux isolés / air-gapped, ces définitions sont livrées
//! avec l'application — aucune requête réseau. Elles restent volontairement
//! courtes et factuelles : de quoi lire un relevé sans être expert, sans
//! prétendre remplacer la spécification d'un protocole.

use std::collections::HashSet;

use icu_normalizer::DecomposingNormalizerBorrowed;

/// Catégorie d'une entrée du glossaire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GlossaryCategory {
    /// « Liaison » (couche 2).
    Liaison,
    /// « Réseau » (couche 3).
    Reseau,
    /// « Transport » (couche 4).
    Transport,
    /// « Application IT ».
    ApplicationIt,
    /// « Application OT/ICS ».
    ApplicationOtIcs,
    /// « Concept ».
    Concept,
}

impl GlossaryCategory {
    /// Libellé affiché de la catégorie.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Liaison => "Liaison",
            Self::Reseau => "Réseau",
            Self::Transport => "Transport",
            Self::ApplicationIt => "Application IT",
            Self::ApplicationOtIcs => "Application OT/ICS",
            Self::Concept => "Concept",
        }
    }

    /// Rang de la catégorie dans [`GLOSSARY_CATEGORY_ORDER`].
    pub fn order_index(self) -> usize {
        GLOSSARY_CATEGORY_ORDER
            .iter()
            .position(|&category| category == self)
            .unwrap_or(usize::MAX)
    }
}

/// Une entrée du glossaire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlossaryEntry {
    /// Terme défini.
    pub term: &'static str,
    /// Formes alternatives (sigles, variantes d'écriture) servant à repérer le
    /// terme dans les libellés de protocole du relevé.
    pub aliases: &'static [&'static str],
    /// Catégorie de l'entrée.
    pub category: GlossaryCategory,
    /// Définition courte, le nom du produit remplacé par [`PRODUCT_TOKEN`] :
    /// lire par [`GlossaryEntry::definition_for`].
    pub definition: &'static str,
}

/// Marqueur du nom du produit dans [`GlossaryEntry::definition`]. Le nom
/// du format (« LUNAR Flow Matrix Standard ») n'est pas marqué.
pub const PRODUCT_TOKEN: &str = "{produit}";

impl GlossaryEntry {
    /// Définition à afficher, `app` (« LUNAR ») à la place de
    /// [`PRODUCT_TOKEN`].
    pub fn definition_for(&self, app: &str) -> String {
        self.definition.replace(PRODUCT_TOKEN, app)
    }
}

/// Ordre d'affichage des catégories : la pile réseau de bas en haut, puis les
/// concepts.
pub const GLOSSARY_CATEGORY_ORDER: [GlossaryCategory; 6] = [
    GlossaryCategory::Liaison,
    GlossaryCategory::Reseau,
    GlossaryCategory::Transport,
    GlossaryCategory::ApplicationIt,
    GlossaryCategory::ApplicationOtIcs,
    GlossaryCategory::Concept,
];

/// Le glossaire, groupé par catégorie.
pub static GLOSSARY: &[GlossaryEntry] = &[
    // ---- Liaison (L2) ------------------------------------------------------
    GlossaryEntry {
        term: "Ethernet",
        aliases: &["eth"],
        category: GlossaryCategory::Liaison,
        definition: "Protocole de couche liaison le plus répandu sur les réseaux locaux. Transporte des trames entre interfaces identifiées par leur adresse MAC.",
    },
    GlossaryEntry {
        term: "Adresse MAC",
        aliases: &["mac"],
        category: GlossaryCategory::Liaison,
        definition: "Identifiant matériel de 48 bits (6 octets) d'une interface réseau. Ses 3 premiers octets (OUI) désignent le fabricant.",
    },
    GlossaryEntry {
        term: "OUI",
        aliases: &["oui"],
        category: GlossaryCategory::Liaison,
        definition: "Organizationally Unique Identifier : les 3 premiers octets d'une MAC, attribués par l'IEEE à un constructeur. Permet de déduire le fabricant d'un équipement.",
    },
    GlossaryEntry {
        term: "VLAN (802.1Q)",
        aliases: &["vlan", "8021q", "dot1q", "vlantaggedframe"],
        category: GlossaryCategory::Liaison,
        definition: "Réseau local virtuel : segmente un réseau physique en domaines de diffusion isolés. Marqué dans la trame par une étiquette 802.1Q portant l'identifiant de VLAN.",
    },
    GlossaryEntry {
        term: "ARP",
        aliases: &["arp"],
        category: GlossaryCategory::Liaison,
        definition: "Address Resolution Protocol : résout une adresse IPv4 en adresse MAC sur le réseau local (« qui a l'IP X ? »), par diffusion. Très bavard en broadcast.",
    },
    GlossaryEntry {
        term: "RARP",
        aliases: &["rarp"],
        category: GlossaryCategory::Liaison,
        definition: "Reverse ARP : retrouve une IP à partir d'une MAC (inverse de l'ARP). Ancien, remplacé par BOOTP/DHCP.",
    },
    GlossaryEntry {
        term: "LLDP",
        aliases: &["lldp"],
        category: GlossaryCategory::Liaison,
        definition: "Link Layer Discovery Protocol : chaque équipement annonce son identité et ses capacités à ses voisins directs. Sert à cartographier la topologie physique.",
    },
    GlossaryEntry {
        term: "GOOSE",
        aliases: &["goose", "iec61850", "61850"],
        category: GlossaryCategory::Liaison,
        definition: "Generic Object Oriented Substation Event (IEC 61850-8-1) : les événements d'un poste électrique (déclenchements, positions d'organes) publiés en multicast directement sur Ethernet (EtherType 0x88B8), souvent sous une étiquette VLAN, sans IP ni transport. Temps réel : quelques millisecondes exigées entre la détection et la réception.",
    },
    GlossaryEntry {
        term: "STP",
        aliases: &["stp", "rstp", "mstp", "spanningtree"],
        category: GlossaryCategory::Liaison,
        definition: "Spanning Tree Protocol : empêche les boucles réseau en désactivant les liens redondants. Trafic de contrôle entre commutateurs (MAC 01:80:c2:...).",
    },
    GlossaryEntry {
        term: "Broadcast",
        aliases: &["broadcast", "diffusion"],
        category: GlossaryCategory::Liaison,
        definition: "Diffusion à tous les hôtes d'un segment. MAC ff:ff:ff:ff:ff:ff, IPv4 255.255.255.255 ou l'adresse de diffusion du sous-réseau.",
    },
    GlossaryEntry {
        term: "Multicast",
        aliases: &["multicast"],
        category: GlossaryCategory::Liaison,
        definition: "Diffusion à un groupe d'hôtes abonnés. MAC 01:00:5e (IPv4) ou 33:33 (IPv6) ; plages IP 224.0.0.0–239.255.255.255 et ff00::/8.",
    },
    GlossaryEntry {
        term: "Unicast",
        aliases: &["unicast"],
        category: GlossaryCategory::Liaison,
        definition: "Communication d'un émetteur vers un unique destinataire : le cas le plus courant.",
    },
    GlossaryEntry {
        term: "LINKTYPE / DLT",
        aliases: &["dlt", "linktype"],
        category: GlossaryCategory::Liaison,
        definition: "Data Link Type : identifie le format de couche liaison d'une capture (Ethernet, RAW IP, Linux SLL…) et donc le décodeur à appliquer à chaque paquet.",
    },
    GlossaryEntry {
        term: "Linux SLL",
        aliases: &["linuxsll", "sll", "sll2", "cooked"],
        category: GlossaryCategory::Liaison,
        definition: "« Cooked capture » de Linux, utilisée en capturant sur l'interface `any` : un pseudo-en-tête remplace l'en-tête Ethernet réel.",
    },
    // ---- Réseau (L3) -------------------------------------------------------
    GlossaryEntry {
        term: "IPv4",
        aliases: &["ipv4", "ip", "minipv4"],
        category: GlossaryCategory::Reseau,
        definition: "Protocole réseau à adresses de 32 bits (ex. 192.168.1.1). Achemine les paquets entre réseaux via le routage.",
    },
    GlossaryEntry {
        term: "IPv6",
        aliases: &["ipv6"],
        category: GlossaryCategory::Reseau,
        definition: "Successeur d'IPv4 à adresses de 128 bits (ex. 2001:db8::1). Espace d'adressage immense, autoconfiguration, NAT non nécessaire.",
    },
    GlossaryEntry {
        term: "ICMP",
        aliases: &["icmp", "icmpv4", "ping"],
        category: GlossaryCategory::Reseau,
        definition: "Internet Control Message Protocol : messages de contrôle et d'erreur d'IPv4 (ping, « destination inaccessible », etc.).",
    },
    GlossaryEntry {
        term: "ICMPv6",
        aliases: &["icmpv6", "ndp"],
        category: GlossaryCategory::Reseau,
        definition: "Version IPv6 d'ICMP. Assure aussi la découverte de voisins (NDP), l'équivalent IPv6 de l'ARP.",
    },
    GlossaryEntry {
        term: "IGMP",
        aliases: &["igmp"],
        category: GlossaryCategory::Reseau,
        definition: "Internet Group Management Protocol : gère l'abonnement des hôtes aux groupes multicast IPv4. Les routeurs s'en servent pour savoir où relayer le trafic multicast.",
    },
    GlossaryEntry {
        term: "TTL",
        aliases: &["ttl", "hoplimit"],
        category: GlossaryCategory::Reseau,
        definition: "Time To Live : compteur décrémenté à chaque routeur traversé ; à zéro le paquet est jeté. Évite les boucles de routage infinies.",
    },
    GlossaryEntry {
        term: "MTU",
        aliases: &["mtu"],
        category: GlossaryCategory::Reseau,
        definition: "Maximum Transmission Unit : taille maximale d'une trame sur un lien (1500 octets en Ethernet standard). Au-delà, fragmentation.",
    },
    GlossaryEntry {
        term: "APIPA / lien-local IPv4",
        aliases: &["apipa", "linklocal", "169254"],
        category: GlossaryCategory::Reseau,
        definition: "Adressage automatique en 169.254.0.0/16 quand aucun serveur DHCP ne répond. Communication locale seulement — souvent le signe d'un problème DHCP.",
    },
    GlossaryEntry {
        term: "Lien-local IPv6",
        aliases: &["fe80", "linklocalipv6"],
        category: GlossaryCategory::Reseau,
        definition: "Adresses fe80::/10 valables uniquement sur le segment local, jamais routées. Utilisées par la découverte de voisins.",
    },
    GlossaryEntry {
        term: "Loopback",
        aliases: &["loopback", "127001", "localhost"],
        category: GlossaryCategory::Reseau,
        definition: "Interface virtuelle de la machine vers elle-même : 127.0.0.1 en IPv4, ::1 en IPv6.",
    },
    GlossaryEntry {
        term: "CIDR / sous-réseau",
        aliases: &["cidr", "sousreseau", "subnet", "masque"],
        category: GlossaryCategory::Reseau,
        definition: "Notation à préfixe (ex. /24) découpant l'espace d'adressage en sous-réseaux. Le préfixe indique le nombre de bits de réseau.",
    },
    GlossaryEntry {
        term: "NAT",
        aliases: &["nat"],
        category: GlossaryCategory::Reseau,
        definition: "Network Address Translation : traduit les adresses privées en une adresse publique partagée à la sortie d'un réseau.",
    },
    // ---- Transport (L4) ----------------------------------------------------
    GlossaryEntry {
        term: "TCP",
        aliases: &["tcp"],
        category: GlossaryCategory::Transport,
        definition: "Transmission Control Protocol : transport fiable, orienté connexion, avec accusés de réception et remise ordonnée. Base de HTTP, TLS, etc.",
    },
    GlossaryEntry {
        term: "UDP",
        aliases: &["udp"],
        category: GlossaryCategory::Transport,
        definition: "User Datagram Protocol : transport léger, sans connexion ni garantie de livraison. Utilisé par DNS, DHCP, le streaming et beaucoup de protocoles industriels.",
    },
    GlossaryEntry {
        term: "Port",
        aliases: &["port"],
        category: GlossaryCategory::Transport,
        definition: "Numéro (0–65535) identifiant un service sur un hôte. Les ports « bien connus » (<1024) désignent des services standard (80 HTTP, 443 HTTPS…).",
    },
    GlossaryEntry {
        term: "Port dynamique / éphémère",
        aliases: &["ephemere", "dynamique", "ephemeral"],
        category: GlossaryCategory::Transport,
        definition: "Ports 49152–65535 attribués temporairement au client d'une connexion. Ils ne désignent pas un service.",
    },
    GlossaryEntry {
        term: "QUIC",
        aliases: &["quic", "quiq"],
        category: GlossaryCategory::Transport,
        definition: "Transport moderne sur UDP, chiffré par défaut (TLS 1.3 intégré) et à faible latence. Base de HTTP/3.",
    },
    GlossaryEntry {
        term: "Handshake TCP",
        aliases: &["handshake", "synack", "troiswayhandshake"],
        category: GlossaryCategory::Transport,
        definition: "Établissement d'une connexion TCP en trois messages (SYN, SYN-ACK, ACK) avant tout échange de données.",
    },
    GlossaryEntry {
        term: "ESP",
        aliases: &["esp", "ipsecesp"],
        category: GlossaryCategory::Transport,
        definition: "Encapsulating Security Payload (RFC 4303) : la charge chiffrée d'un tunnel IPsec, protocole IP 50 ou encapsulée en UDP 4500 derrière un NAT. Seuls l'index de sécurité (SPI) et le numéro de séquence sont lisibles ; le contenu et son protocole sont opaques.",
    },
    // ---- Application IT ----------------------------------------------------
    GlossaryEntry {
        term: "HTTP",
        aliases: &["http"],
        category: GlossaryCategory::ApplicationIt,
        definition: "HyperText Transfer Protocol : protocole du Web (port 80), requêtes/réponses en clair.",
    },
    GlossaryEntry {
        term: "TLS / HTTPS",
        aliases: &["tls", "ssl", "https"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Transport Layer Security : chiffre et authentifie une communication (successeur de SSL). HTTPS = HTTP sur TLS (port 443).",
    },
    GlossaryEntry {
        term: "DNS",
        aliases: &["dns"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Domain Name System : traduit les noms de domaine en adresses IP (port 53), souvent en UDP.",
    },
    GlossaryEntry {
        term: "mDNS",
        aliases: &["mdns", "bonjour", "zeroconf"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Multicast DNS : résolution de noms sans serveur, en multicast local (224.0.0.251, port 5353). Base de Bonjour/AirPlay et de nombreux objets connectés.",
    },
    GlossaryEntry {
        term: "LLMNR",
        aliases: &["llmnr"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Link-Local Multicast Name Resolution : résolution de noms locale de secours sous Windows (port 5355). Souvent désactivée pour raisons de sécurité.",
    },
    GlossaryEntry {
        term: "DHCP",
        aliases: &["dhcp", "bootp"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Dynamic Host Configuration Protocol : attribue automatiquement adresse IP, passerelle et DNS aux hôtes (ports 67/68).",
    },
    GlossaryEntry {
        term: "DHCPv6",
        aliases: &["dhcpv6"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Version IPv6 de DHCP (ports 546/547).",
    },
    GlossaryEntry {
        term: "NTP",
        aliases: &["ntp"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Network Time Protocol : synchronise l'horloge des machines (port 123). Essentiel pour corréler les journaux.",
    },
    GlossaryEntry {
        term: "SNMP",
        aliases: &["snmp"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Simple Network Management Protocol : supervision et configuration des équipements réseau (ports 161/162).",
    },
    GlossaryEntry {
        term: "MQTT",
        aliases: &["mqtt"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Messagerie publish/subscribe légère (port 1883), très répandue en IoT et en télémétrie industrielle.",
    },
    GlossaryEntry {
        term: "PostgreSQL",
        aliases: &["postgresql", "postgres"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Protocole de la base de données PostgreSQL (port 5432).",
    },
    GlossaryEntry {
        term: "SLP / SRVLOC",
        aliases: &["slp", "srvloc", "srvlock"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Service Location Protocol : découverte de services sur un réseau local (port 427).",
    },
    GlossaryEntry {
        term: "GIOP",
        aliases: &["giop", "iiop", "corba"],
        category: GlossaryCategory::ApplicationIt,
        definition: "General Inter-ORB Protocol : transport de CORBA pour les objets distribués.",
    },
    GlossaryEntry {
        term: "Java RMI",
        aliases: &["javarmi", "rmi", "jrmp"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Java Remote Method Invocation : appel de méthodes entre machines virtuelles Java (registre sur TCP 1099 par défaut). Les objets échangés sont sérialisés, en clair.",
    },
    GlossaryEntry {
        term: "SIP",
        aliases: &["sip"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Session Initiation Protocol (RFC 3261) : signalisation des appels de téléphonie et de visioconférence sur IP (port 5060, UDP ou TCP). Il établit et termine les appels ; la voix elle-même passe par un autre flux. En clair : numéros appelés et identifiants sont lisibles.",
    },
    GlossaryEntry {
        term: "NetBIOS",
        aliases: &["netbios", "nbns", "nbss", "nbt"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Ancienne API réseau Windows pour le partage de fichiers et la résolution de noms locale (ports 137–139). Souvent bavarde en broadcast.",
    },
    GlossaryEntry {
        term: "SMB",
        aliases: &["smb", "cifs", "microsoftds"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Server Message Block : partage de fichiers et d'imprimantes Windows (port 445).",
    },
    GlossaryEntry {
        term: "SSDP / UPnP",
        aliases: &["ssdp", "upnp"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Découverte d'équipements UPnP en multicast (239.255.255.250, port 1900). Fréquent chez les objets grand public.",
    },
    GlossaryEntry {
        term: "LDAP",
        aliases: &["ldap", "ldaps"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Protocole d'annuaire (port 389, ou 636 en TLS) au cœur d'Active Directory.",
    },
    GlossaryEntry {
        term: "Kerberos",
        aliases: &["kerberos"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Protocole d'authentification à tickets (port 88), utilisé par Active Directory.",
    },
    GlossaryEntry {
        term: "SSH",
        aliases: &["ssh"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Secure Shell : administration distante chiffrée (port 22).",
    },
    GlossaryEntry {
        term: "RDP",
        aliases: &["rdp", "msrdpbcgr"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Remote Desktop Protocol : bureau à distance Windows (TCP 3389). L'étiquette couvre l'ouverture de connexion X.224 (demande et confirmation, négociation de la sécurité) ; la session elle-même bascule le plus souvent en TLS.",
    },
    GlossaryEntry {
        term: "RADIUS",
        aliases: &["radius", "aaa"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Remote Authentication Dial-In User Service (RFC 2865) : authentification, autorisation et comptabilité centralisées des accès (Wi-Fi, 802.1X, VPN, administration). UDP 1812 pour l'authentification, 1813 pour la comptabilité ; le mot de passe est masqué, le reste circule en clair.",
    },
    GlossaryEntry {
        term: "IKE",
        aliases: &[
            "ike",
            "ikev1",
            "ikev2",
            "isakmp",
            "ipsec",
            "natkeepalive",
            "natt",
        ],
        category: GlossaryCategory::ApplicationIt,
        definition: "Internet Key Exchange (ISAKMP, RFC 7296) : la négociation des clés et des associations de sécurité d'un tunnel IPsec, sur UDP 500 puis 4500 derrière un NAT (NAT-T, RFC 3948). Sur le port 4500 voyagent aussi les paquets ESP encapsulés et le NAT-keepalive d'un octet qui maintient la translation ouverte.",
    },
    GlossaryEntry {
        term: "WireGuard",
        aliases: &["wireguard", "wg"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Tunnel VPN moderne sur UDP (port 51820 par défaut) : quatre types de messages (initiation, réponse, cookie, données) à l'en-tête fixe. Tout le contenu est chiffré ; {produit} ne voit que les pairs et le volume. Utilisé tel quel ou derrière Tailscale, Mullvad, NordVPN…",
    },
    GlossaryEntry {
        term: "Syslog",
        aliases: &["syslog"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Transport standard des journaux d'événements (port 514).",
    },
    GlossaryEntry {
        term: "FTP",
        aliases: &["ftp"],
        category: GlossaryCategory::ApplicationIt,
        definition: "File Transfer Protocol : transfert de fichiers en clair (port 21 pour le contrôle). Identifiants et contenu lisibles sur le réseau ; encore courant pour pousser des recettes ou des sauvegardes vers des équipements industriels.",
    },
    GlossaryEntry {
        term: "SMTP",
        aliases: &["smtp"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Simple Mail Transfer Protocol : envoi de courrier électronique (ports 25, 587). Sur un réseau industriel, souvent des alarmes ou rapports envoyés par un équipement.",
    },
    GlossaryEntry {
        term: "NNTP",
        aliases: &["nntp"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Network News Transfer Protocol : diffusion de forums Usenet (port 119). Rare aujourd'hui ; sa présence sur un réseau local mérite d'être expliquée.",
    },
    GlossaryEntry {
        term: "OpenVPN",
        aliases: &["openvpn"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Tunnel VPN chiffré, sur UDP ou TCP (port 1194 par défaut). Le trafic transporté est illisible : {produit} ne voit que les deux extrémités du tunnel.",
    },
    GlossaryEntry {
        term: "Bitcoin",
        aliases: &["bitcoin"],
        category: GlossaryCategory::ApplicationIt,
        definition: "Protocole pair-à-pair du réseau Bitcoin (port 8333). Inattendu sur un réseau d'entreprise ou industriel : à rapprocher d'un usage non déclaré.",
    },
    // ---- Application OT/ICS ------------------------------------------------
    GlossaryEntry {
        term: "Modbus/TCP",
        aliases: &["modbus", "modbustcp", "modbusudp"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Protocole industriel très répandu (port 502) pour lire/écrire les registres d'un automate. Simple, sans authentification native. La même enveloppe circule parfois sur UDP, hors spécification : elle est alors étiquetée ModbusUDP.",
    },
    GlossaryEntry {
        term: "UMAS",
        aliases: &["umas"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Protocole propriétaire de Schneider Electric pour piloter les automates Modicon (M340, M580, Quantum, Premium). Il voyage dans une enveloppe Modbus/TCP (port 502) sous le code fonction 0x5A réservé au constructeur : une trame UMAS est donc aussi une trame Modbus, et c'est UMAS qui est retenu. Sans spécification publique ni authentification.",
    },
    GlossaryEntry {
        term: "PROFINET",
        aliases: &["profinet", "profinetrt", "pnio"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Protocole industriel Ethernet de Siemens pour l'automatisation temps réel.",
    },
    GlossaryEntry {
        term: "PTP",
        aliases: &["ptp", "ptpv2", "ieee1588"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Precision Time Protocol (IEEE 1588) : synchronise les horloges d'un réseau à la microseconde près, voire mieux, là où NTP s'arrête à la milliseconde. Il circule sur UDP (ports 319 et 320) ou directement sur Ethernet (EtherType 0x88F7). On le trouve dans les postes électriques (IEC 61850, C37.238), l'automatisme, les télécoms et les réseaux embarqués temps réel (802.1AS).",
    },
    GlossaryEntry {
        term: "OPC UA",
        aliases: &["opcua"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Standard moderne et sécurisé d'échange de données industrielles, indépendant du constructeur (port 4840).",
    },
    GlossaryEntry {
        term: "ASTERIX",
        aliases: &["asterix"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Format d'échange des données de surveillance du contrôle aérien (EUROCONTROL-SPEC-0149) : plots et pistes radar (CAT 048), messages de service du radar (CAT 034) et ADS-B (CAT 021). Sans port réservé ni magic, il est reconnu par sa seule structure : un datagramme UDP n'est étiqueté que si tous ses data blocks se découpent exactement selon leur UAP.",
    },
    GlossaryEntry {
        term: "S7comm",
        aliases: &["s7comm", "s7", "s7commplus"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Protocole propriétaire Siemens pour dialoguer avec les automates S7, transporté sur ISO-TSAP/COTP (port 102).",
    },
    GlossaryEntry {
        term: "COTP",
        aliases: &["cotp", "copt", "isotsap", "tpkt"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Connection-Oriented Transport Protocol (ISO 8073) : couche transport ISO utilisée sous S7comm (port 102).",
    },
    GlossaryEntry {
        term: "EtherNet/IP",
        aliases: &["ethernetip", "enip", "cip"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Protocole industriel (CIP sur Ethernet) porté par Rockwell/ODVA (ports 44818 et 2222).",
    },
    GlossaryEntry {
        term: "ADS/AMS",
        aliases: &["ads", "ams", "beckhoff", "twincat"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Protocole de Beckhoff (TwinCAT) pour l'automatisation.",
    },
    GlossaryEntry {
        term: "DNP3",
        aliases: &["dnp3", "dnp"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Distributed Network Protocol : télégestion courante dans l'énergie et l'eau (port 20000).",
    },
    GlossaryEntry {
        term: "Valmet DNA",
        aliases: &["valmetdna", "valmet", "metsodna"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Réseau de process des systèmes de contrôle-commande distribués Valmet DNA (ex-Metso DNA). Protocole propriétaire, sans spécification ni dissecteur publics : il est reconnu par sa seule structure, observée sur le trafic. Chaque nœud diffuse en multicast (UDP 2517, groupe 239.0.171.1) un télégramme d'état périodique et des messages à étiquette ; une balise horaire circule d'UDP 4516 vers 4517.",
    },
    GlossaryEntry {
        term: "RTPS",
        aliases: &["rtps", "dds", "ddsi"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Real-Time Publish-Subscribe (OMG DDSI-RTPS) : le protocole filaire de DDS, bus de données temps réel des systèmes embarqués, de la robotique (ROS 2) et des systèmes d'armes. Découverte par multicast (UDP 7400) puis échanges unicast ; chaque message porte le vendeur de l'implémentation.",
    },
    GlossaryEntry {
        term: "DIS",
        aliases: &["dis", "ieee1278"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Distributed Interactive Simulation (IEEE 1278.1) : les PDU des simulations d'entraînement distribuées (état d'entités, tirs, détonations, émissions radio et voix), diffusées en UDP (port 3000 par défaut) entre simulateurs. Sans en-tête d'application : reconnu par la structure fixe de son en-tête de PDU.",
    },
    GlossaryEntry {
        term: "STANAG 5066",
        aliases: &["stanag5066", "stanag", "sis"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Profil OTAN des communications de données HF : l'interface SIS (Subnet Interface Sublayer, TCP 5066) relie une application à un modem HF. Primitives d'ouverture, de dépôt et de remise de messages ; les octets utiles transitent vers la radio.",
    },
    GlossaryEntry {
        term: "IEC 60870-5-104",
        aliases: &["iec104", "60870", "iec608705104"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Protocole de téléconduite des réseaux électriques (port 2404).",
    },
    GlossaryEntry {
        term: "BACnet",
        aliases: &["bacnet", "bacnetip", "bvlc"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Protocole de gestion technique du bâtiment (CVC, éclairage, contrôle d'accès), port 47808.",
    },
    GlossaryEntry {
        term: "SCADA",
        aliases: &["scada"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Supervisory Control And Data Acquisition : systèmes de supervision et de pilotage des procédés industriels.",
    },
    GlossaryEntry {
        term: "Automate (PLC / API)",
        aliases: &["plc", "api", "automate"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Automate programmable industriel : calculateur temps réel pilotant capteurs et actionneurs d'un procédé.",
    },
    GlossaryEntry {
        term: "ICS / OT",
        aliases: &["ics", "ot"],
        category: GlossaryCategory::ApplicationOtIcs,
        definition: "Industrial Control Systems / Operational Technology : systèmes informatiques qui pilotent des procédés physiques (usines, énergie, eau).",
    },
    // ---- Concepts ---------------------------------------------------------
    GlossaryEntry {
        term: "LFMS",
        aliases: &["lfms"],
        category: GlossaryCategory::Concept,
        definition: "LUNAR Flow Matrix Standard : le format de matrice de flux de {produit} (colonnes, préambule versionné, provenance). Voir LFMS.md.",
    },
    GlossaryEntry {
        term: "Matrice de flux",
        aliases: &["matrice", "flowmatrix"],
        category: GlossaryCategory::Concept,
        definition: "Tableau des conversations observées (source, destination, protocole, ports, volumétrie). C'est le cœur d'un relevé {produit}.",
    },
    GlossaryEntry {
        term: "Capture passive",
        aliases: &["passive", "passif"],
        category: GlossaryCategory::Concept,
        definition: "Observation du trafic sans rien émettre sur le réseau. Indispensable en OT et sur les réseaux sensibles.",
    },
    GlossaryEntry {
        term: "Mode promiscuité",
        aliases: &["promiscuite", "promiscuous", "promisc"],
        category: GlossaryCategory::Concept,
        definition: "Mode où l'interface capture toutes les trames qu'elle voit, pas seulement celles qui lui sont destinées.",
    },
    GlossaryEntry {
        term: "PCAP / PCAPNG",
        aliases: &["pcap", "pcapng"],
        category: GlossaryCategory::Concept,
        definition: "Formats de fichiers de capture réseau (paquets bruts horodatés), lisibles par Wireshark, tcpdump et {produit}.",
    },
    GlossaryEntry {
        term: "Port miroir / SPAN",
        aliases: &["span", "portmiroir", "mirror"],
        category: GlossaryCategory::Concept,
        definition: "Fonction d'un commutateur qui recopie le trafic d'un ou plusieurs ports vers un port d'analyse.",
    },
    GlossaryEntry {
        term: "TAP réseau",
        aliases: &["tap"],
        category: GlossaryCategory::Concept,
        definition: "Boîtier passif inséré sur un lien pour en dupliquer le trafic vers un analyseur, sans perturber le flux.",
    },
    GlossaryEntry {
        term: "Tunnel / CAPWAP",
        aliases: &["tunnel", "capwap", "encapsulation", "encap"],
        category: GlossaryCategory::Concept,
        definition: "Encapsulation d'un trafic dans un autre (ex. CAPWAP pour les bornes Wi-Fi). {produit} décapsule et relie père/fils par un identifiant de tunnel.",
    },
    GlossaryEntry {
        term: "Shadow IT/OT",
        aliases: &["shadow", "shadowit", "shadowot"],
        category: GlossaryCategory::Concept,
        definition: "Équipements ou services présents sur le réseau mais non déclarés ou non autorisés. L'un des cas d'usage de {produit}.",
    },
    GlossaryEntry {
        term: "Endpoint",
        aliases: &["endpoint", "noeud", "hote"],
        category: GlossaryCategory::Concept,
        definition: "Équipement identifié dans le relevé (par IP/MAC/VLAN) : un nœud du graphe.",
    },
    GlossaryEntry {
        term: "Flux",
        aliases: &["flux", "flow", "conversation"],
        category: GlossaryCategory::Concept,
        definition: "Une conversation entre deux endpoints pour un protocole donné : une arête du graphe.",
    },
];

/// Normalise un terme pour la comparaison : minuscules, sans accents ni
/// séparateurs. « Modbus/TCP » et « modbus tcp » donnent le même jeton.
///
/// Comme `value.normalize("NFD").replace(/\p{Diacritic}/gu, "")
/// .toLowerCase().replace(/[^a-z0-9]/g, "")` : après décomposition, seules
/// les lettres et chiffres ASCII survivent, ce qui écarte aussi les
/// diacritiques.
pub fn normalize_token(value: &str) -> String {
    DecomposingNormalizerBorrowed::new_nfd()
        .normalize(value)
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        .collect()
}

/// Jetons normalisés d'une entrée (terme + alias), pour la mise en
/// correspondance avec ce qui est observé dans le relevé.
pub fn entry_tokens(entry: &GlossaryEntry) -> Vec<String> {
    std::iter::once(entry.term)
        .chain(entry.aliases.iter().copied())
        .map(normalize_token)
        .filter(|token| !token.is_empty())
        .collect()
}

/// Entrées du glossaire dont au moins un jeton figure dans l'ensemble observé
/// (protocoles et signaux du relevé). Conserve l'ordre de [`GLOSSARY`].
pub fn match_glossary(observed: &HashSet<String>) -> Vec<&'static GlossaryEntry> {
    GLOSSARY
        .iter()
        .filter(|entry| {
            entry_tokens(entry)
                .iter()
                .any(|token| observed.contains(token))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::investigation::test_support::PARSER_APPLICATION_LABELS;

    fn set(tokens: &[&str]) -> HashSet<String> {
        tokens.iter().map(|token| (*token).to_string()).collect()
    }

    #[test]
    fn normalize_token_minuscule_sans_accent_ni_separateur() {
        assert_eq!(normalize_token("Modbus/TCP"), "modbustcp");
        assert_eq!(normalize_token("OPC UA"), "opcua");
        assert_eq!(normalize_token("Réseau"), "reseau");
        assert_eq!(normalize_token("EtherNet/IP"), "ethernetip");
    }

    #[test]
    fn match_glossary_ne_renvoie_que_les_entrees_observees() {
        let terms: Vec<_> = match_glossary(&set(&["tls", "modbustcp", "arp"]))
            .iter()
            .map(|entry| entry.term)
            .collect();
        assert!(terms.contains(&"TLS / HTTPS"), "TLS attendu");
        assert!(terms.contains(&"Modbus/TCP"), "Modbus attendu");
        assert!(terms.contains(&"ARP"), "ARP attendu");
        assert!(
            !terms.contains(&"IGMP"),
            "IGMP non observé ne doit pas apparaître"
        );
    }

    #[test]
    fn match_glossary_ensemble_vide_aucune_entree() {
        assert_eq!(match_glossary(&HashSet::new()).len(), 0);
    }

    #[test]
    fn glossaire_igmp_present_et_consultable_meme_hors_releve() {
        let igmp = GLOSSARY
            .iter()
            .find(|entry| entry.term == "IGMP")
            .expect("entrée IGMP absente");
        assert!(
            igmp.definition.chars().count() > 20,
            "définition IGMP trop courte"
        );
        assert!(
            entry_tokens(igmp).contains(&"igmp".to_string()),
            "jeton igmp manquant"
        );
    }

    #[test]
    fn nom_du_produit_marque_sauf_dans_le_nom_du_format() {
        for entry in GLOSSARY {
            assert!(
                !entry
                    .definition
                    .replace("LUNAR Flow Matrix Standard", "")
                    .contains("LUNAR"),
                "{} : nom du produit écrit en dur",
                entry.term
            );
        }
        let format = GLOSSARY
            .iter()
            .find(|entry| entry.term == "LFMS")
            .map(|entry| entry.definition_for("LUNAR"));
        assert!(format.is_some_and(|text| {
            text.starts_with("LUNAR Flow Matrix Standard : le format de matrice de flux de LUNAR ")
        }));
        let tunnel = GLOSSARY
            .iter()
            .find(|entry| entry.definition.contains("décapsule"))
            .map(|entry| entry.definition_for("LUNAR"));
        assert!(tunnel.is_some_and(|text| text.contains("LUNAR décapsule")));
    }

    #[test]
    fn glossaire_integrite_liste_fournie_categories_connues_definitions_non_vides() {
        assert!(GLOSSARY.len() >= 50, "glossaire attendu conséquent (>=50)");
        for entry in GLOSSARY {
            assert!(!entry.term.trim().is_empty(), "terme vide");
            assert!(
                !entry.definition.trim().is_empty(),
                "définition vide pour {}",
                entry.term
            );
            assert!(
                !entry_tokens(entry).is_empty(),
                "aucun jeton pour {}",
                entry.term
            );
            assert!(
                GLOSSARY_CATEGORY_ORDER.contains(&entry.category),
                "catégorie inconnue pour {}",
                entry.term
            );
        }
    }

    #[test]
    fn glossaire_tout_protocole_applicatif_emis_par_le_parser_a_une_definition() {
        let missing: Vec<_> = PARSER_APPLICATION_LABELS
            .iter()
            .filter(|label| match_glossary(&set(&[&normalize_token(label)])).is_empty())
            .collect();
        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn glossaire_95_entrees() {
        assert_eq!(GLOSSARY.len(), 95);
    }
}
