//! Correspondance port → service *probable*, d'après le registre des ports
//! bien connus (IANA) et les usages courants IT et OT/ICS. C'est une
//! **indication**, pas une preuve : un port n'impose pas le protocole qui y
//! circule. La vue présente donc ces noms comme « probables ». LUNAR ne
//! conclut pas — il aide à lire.
//!
//! Volontairement centré sur ce qui parle à un audit réseau/OT : services
//! d'infrastructure, administration, et protocoles industriels (Modbus, S7,
//! EtherNet/IP, PROFINET, DNP3, IEC-104, BACnet, OPC UA…).

/// Service *probable* d'un port, ou chaîne vide si le port n'est pas
/// répertorié (la vue affiche alors « — »). Ne devine jamais : renvoie
/// seulement ce que le registre des ports bien connus indique.
pub fn port_service(port: u16) -> &'static str {
    match port {
        20 => "FTP (données)",
        21 => "FTP",
        22 => "SSH",
        23 => "Telnet",
        25 => "SMTP",
        37 => "Time",
        42 => "WINS",
        43 => "WHOIS",
        49 => "TACACS+",
        53 => "DNS",
        67 => "DHCP (serveur)",
        68 => "DHCP (client)",
        69 => "TFTP",
        79 => "Finger",
        80 => "HTTP",
        88 => "Kerberos",
        102 => "S7comm / ISO-TSAP (COTP)",
        110 => "POP3",
        111 => "ONC RPC / Portmap",
        113 => "Ident",
        119 => "NNTP",
        123 => "NTP",
        135 => "MS RPC (EPMAP)",
        137 => "NetBIOS (nom)",
        138 => "NetBIOS (datagramme)",
        139 => "NetBIOS (session)",
        143 => "IMAP",
        161 => "SNMP",
        162 => "SNMP Trap",
        177 => "XDMCP",
        179 => "BGP",
        194 => "IRC",
        319 => "PTP (événements)",
        320 => "PTP (général)",
        389 => "LDAP",
        427 => "SLP (SRVLOC)",
        443 => "HTTPS (TLS)",
        445 => "SMB (Microsoft-DS)",
        464 => "Kerberos (kpasswd)",
        465 => "SMTPS",
        500 => "ISAKMP / IKE",
        502 => "Modbus/TCP",
        514 => "Syslog",
        515 => "LPD (imprimante)",
        520 => "RIP",
        521 => "RIPng",
        523 => "IBM DB2",
        546 => "DHCPv6 (client)",
        547 => "DHCPv6 (serveur)",
        548 => "AFP",
        554 => "RTSP",
        587 => "SMTP (soumission)",
        593 => "MS RPC over HTTP",
        623 => "IPMI / RMCP",
        631 => "IPP (imprimante)",
        636 => "LDAPS",
        646 => "LDP (MPLS)",
        830 => "NETCONF",
        853 => "DNS over TLS",
        873 => "rsync",
        902 => "VMware ESXi",
        989 => "FTPS (données)",
        990 => "FTPS",
        993 => "IMAPS",
        995 => "POP3S",
        1080 => "SOCKS",
        1099 => "Java RMI",
        1194 => "OpenVPN",
        1311 => "Dell OpenManage",
        1433 => "Microsoft SQL Server",
        1434 => "MSSQL Monitor",
        1521 => "Oracle DB",
        1701 => "L2TP",
        1723 => "PPTP",
        1755 => "MMS (streaming)",
        1812 => "RADIUS (auth)",
        1813 => "RADIUS (compta)",
        1883 => "MQTT",
        1900 => "SSDP / UPnP",
        1911 => "Niagara Fox (Tridium)",
        1962 => "PCWorx",
        2000 => "Cisco SCCP",
        2049 => "NFS",
        2222 => "EtherNet/IP",
        2375 => "Docker",
        2376 => "Docker (TLS)",
        2404 => "IEC 60870-5-104",
        2427 => "MGCP",
        2517 => "Valmet DNA (réseau de process)",
        3000 => "DIS (simulation distribuée)",
        3128 => "Squid Proxy",
        3268 => "LDAP (catalogue global)",
        3269 => "LDAP GC (SSL)",
        3306 => "MySQL",
        3389 => "RDP",
        3690 => "Subversion",
        4500 => "IPsec NAT-T",
        4516 => "Valmet DNA (balise horaire)",
        4517 => "Valmet DNA (balise horaire)",
        4840 => "OPC UA",
        4843 => "OPC UA (TLS)",
        4911 => "Niagara Fox (TLS)",
        5006 => "Mitsubishi MELSEC",
        5007 => "Mitsubishi MELSEC",
        5066 => "STANAG 5066 SIS",
        5060 => "SIP",
        5061 => "SIP (TLS)",
        5222 => "XMPP",
        5353 => "mDNS",
        5355 => "LLMNR",
        5432 => "PostgreSQL",
        5601 => "Kibana",
        5672 => "AMQP",
        5683 => "CoAP",
        5684 => "CoAP (DTLS)",
        5900 => "VNC",
        5938 => "TeamViewer",
        5985 => "WinRM (HTTP)",
        5986 => "WinRM (HTTPS)",
        6000 => "X11",
        6379 => "Redis",
        6443 => "Kubernetes API",
        6667 => "IRC",
        7400 => "RTPS / DDS (découverte)",
        7401 => "RTPS / DDS (données)",
        7547 => "TR-069 (CWMP)",
        8000 => "HTTP (alt)",
        8006 => "Proxmox VE",
        8080 => "HTTP (proxy/alt)",
        8086 => "InfluxDB",
        8123 => "Home Assistant",
        8443 => "HTTPS (alt)",
        8883 => "MQTT (TLS)",
        9042 => "Cassandra",
        9092 => "Kafka",
        9100 => "Imprimante (RAW/JetDirect)",
        9200 => "Elasticsearch",
        9600 => "OMRON FINS",
        10000 => "Webmin",
        11211 => "Memcached",
        18245 => "GE SRTP",
        18246 => "GE SRTP",
        20000 => "DNP3",
        20547 => "ProConOS",
        27017 => "MongoDB",
        34962 => "PROFINET (RT)",
        34963 => "PROFINET (RT)",
        34964 => "PROFINET (contexte)",
        44818 => "EtherNet/IP (explicite)",
        47808 => "BACnet/IP",
        51820 => "WireGuard",
        62541 => "OPC UA (alt)",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_service_ports_it_bien_connus() {
        assert_eq!(port_service(138), "NetBIOS (datagramme)");
        assert_eq!(port_service(53), "DNS");
        assert_eq!(port_service(443), "HTTPS (TLS)");
        assert_eq!(port_service(445), "SMB (Microsoft-DS)");
    }

    #[test]
    fn port_service_ports_ot_ics() {
        assert_eq!(port_service(502), "Modbus/TCP");
        assert_eq!(port_service(102), "S7comm / ISO-TSAP (COTP)");
        assert_eq!(port_service(44818), "EtherNet/IP (explicite)");
        assert_eq!(port_service(47808), "BACnet/IP");
        assert_eq!(port_service(20000), "DNP3");
    }

    #[test]
    fn port_service_port_non_repertorie_chaine_vide() {
        assert_eq!(port_service(64123), "");
    }
}
