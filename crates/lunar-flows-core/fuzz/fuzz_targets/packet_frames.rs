//! Cible de fuzzing : une trame hostile reçue en capture directe.
#![no_main]

use libfuzzer_sys::fuzz_target;
use lunar_flows_core::packet::CapturedPacket;
use packet_parser::LinkType;

// Le premier octet choisit un type de liaison pris en charge, le reste est la
// trame brute. Contrairement à `pcap_reader`, ce chemin ne traverse ni
// libpcap ni un format de fichier : il attaque directement les décodeurs de
// `packet_parser`, puis le dépliage des tunnels en niveaux de flux, comme une
// trame forgée arrivant sur l'interface écoutée. Une trame refusée est un
// résultat normal ; seuls un panic, un crash, une erreur de sanitizer ou un
// blocage intéressent le fuzzer.
fuzz_target!(|data: &[u8]| {
    let Some((&link_byte, frame)) = data.split_first() else {
        return;
    };
    let link_type = LinkType(u32::from(link_byte));
    if !packet_parser::parse::is_supported(link_type) {
        return;
    }
    let Ok(flow) = packet_parser::parse::parse(link_type, frame) else {
        return;
    };
    let length = u32::try_from(frame.len()).unwrap_or(u32::MAX);
    let header = pcap::PacketHeader {
        ts: libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        },
        caplen: length,
        len: length,
    };
    for owned in CapturedPacket::from_pcap(&header, flow).to_owned_packets() {
        std::hint::black_box(&owned);
    }
});
