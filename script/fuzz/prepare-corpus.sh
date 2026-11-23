#!/usr/bin/env bash
# Prépare le corpus initial de chaque cible de fuzzing à partir des fixtures
# versionnées. Le corpus n'est pas commité (voir
# crates/lunar-flows-core/fuzz/README.md) : il se reconstruit ici, graines de
# régression comprises. Un corpus déjà enrichi par une campagne locale est
# complété, jamais vidé.
set -Eeuo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixtures="$root/fixtures"
tshark="$fixtures/pcaps/import/pcap_tshark_corpus"
fuzz="$root/crates/lunar-flows-core/fuzz"
pcap_corpus="$fuzz/corpus/pcap_reader"
matrix_corpus="$fuzz/corpus/matrix_reader"
winaudit_corpus="$fuzz/corpus/winaudit_reader"
frames_corpus="$fuzz/corpus/packet_frames"
xlsx_corpus="$fuzz/corpus/xlsx_reader"
mkdir -p -- "$pcap_corpus" "$matrix_corpus" "$winaudit_corpus" "$frames_corpus" "$xlsx_corpus"

# --- pcap_reader ------------------------------------------------------------
# Petites captures : Ethernet, VLAN, IP nu, boucle locale, tunnels (CAPWAP,
# GTP-U, WireGuard), protocoles industriels, PCAPNG, et la frontière des types
# de liaison non pris en charge (IEEE 802.11).
for capture in \
  loopback_null.pcap unsupported_ieee80211.pcapng capwap_management.pcap \
  gtpu_ipv6.pcap capwap_data.pcap vlan.pcap industrial_ethernet.pcap \
  stanag5066_expedited.pcap raw_ip.pcap goose_l2.pcap capwap_radius.pcap \
  ptp_ipv6.pcap bacnet_ip.pcap ike_v1_main_mode.pcap radius.pcap \
  ptp_udp_l2.pcap gtpu_gn.pcap wireguard.pcap
do
  cp -- "$tshark/$capture" "$pcap_corpus/$capture"
done
cp -- "$fixtures/eth_dns.pcapng" "$fixtures/raw_ip.pcapng" "$pcap_corpus/"

# Début d'une capture pcap classique petit-boutiste : en-tête global et
# premiers paquets entiers, dans la limite donnée en octets. Les seules
# captures Linux « cooked » versionnées pèsent près d'un mégaoctet : leurs
# premiers paquets suffisent à amorcer les décodeurs SLL et SLL2.
pcap_head() {
  local source="$1" destination="$2" limit="$3" offset=24 size length
  size="$(wc -c <"$source")"
  while ((offset + 16 <= size)); do
    length="$(od -A n -t u4 -j "$((offset + 8))" -N 4 "$source" | tr -d ' ')"
    ((offset + 16 + length <= limit)) || break
    offset=$((offset + 16 + length))
  done
  head -c "$offset" -- "$source" >"$destination"
}
pcap_head "$tshark/linux_sll.pcap" "$pcap_corpus/linux_sll_head.pcap" 4096
pcap_head "$tshark/linux_sll2.pcap" "$pcap_corpus/linux_sll2_head.pcap" 4096

# Deux entrées malformées déterministes : en-tête global incomplet et dernier
# enregistrement tronqué.
head -c 12 -- "$tshark/vlan.pcap" >"$pcap_corpus/truncated_global_header.pcap"
vlan_size="$(wc -c <"$tshark/vlan.pcap")"
((vlan_size > 8)) || { echo "fixture vlan.pcap trop petite" >&2; exit 1; }
head -c "$((vlan_size - 8))" -- "$tshark/vlan.pcap" >"$pcap_corpus/truncated_packet.pcap"

# --- packet_frames ----------------------------------------------------------
# Trames individuelles des captures pcap classiques du corpus TShark et des
# fixtures, chacune précédée de l'octet de son type de liaison : au plus 40
# par capture. Les pcapng ne donnent rien ici (lus par `pcap_reader`).
find "$tshark" "$fixtures" -maxdepth 1 -type f -name '*.pcap' -print0 | sort -z |
  xargs -0 python3 "$root/script/fuzz/extract-frames.py" "$frames_corpus"

# --- matrix_reader ----------------------------------------------------------
header="mac_source,mac_destination,vlan_id,protocol_data_link,ip_source,ip_source_type,label_source,ip_destination,ip_destination_type,label_destination,port_source,port_destination,protocol_transport,application_protocol,count,total_bytes,last_seen,encap_id,origin"

# Fixtures : export antérieur au préambule, préambule v1, colonne encap_id.
cp -- "$fixtures/20260703_NP_Matrice.csv" "$matrix_corpus/sans_preambule.csv"
cp -- "$fixtures/20260730_DR_wtp.csv" "$matrix_corpus/preambule_v1.csv"
head -n 2 -- "$fixtures/20260730_DR_wtp.csv" >"$matrix_corpus/en_tete_seul.csv"
head -n 40 -- "$fixtures/20260703_NP_Labels_encaps.csv" >"$matrix_corpus/tunnels.csv"

# Préambule v2 complet (contexte percent-encodé, période), VLAN, label à
# préfixe anti-formule, comptes par tunnel et origines multiples.
{
  printf '#LFMS version=2 dlt=ETHERNET site=Usine%%20Nord sensor=sonde%%3DA interface=eth0 first_seen=2026-07-03T15:09:34.000000Z last_seen=2026-07-03T15:09:40.123456Z\n'
  printf '%s\n' "$header"
  printf "e0:d5:5e:28:9b:d4,44:15:24:20:a5:64,12,IPv4,192.168.1.15,Privée,'=1+1,192.168.1.254,Privée,passerelle,58142,443,TCP,TLS,5,516,2026-07-03 15:09:40.123456 UTC,8568e7dc25cf8676:3|0123456789abcdef:1,site-a.csv|site-b.csv\n"
  printf 'c4:64:13:8f:9e:00,ff:ff:ff:ff:ff:ff,,ARP,172.24.8.29,Privée,,172.24.8.29,Privée,,,,,,39,1092,N/A,8568e7dc25cf8676,\n'
} >"$matrix_corpus/preambule_v2.csv"

# Une matrice par type de liaison reconstructible hors Ethernet.
matrix_seed() {
  local name="$1" dlt="$2" row="$3"
  printf '#LFMS version=2 dlt=%s\n%s\n%s\n' "$dlt" "$header" "$row" >"$matrix_corpus/$name"
}
matrix_seed raw.csv RAW \
  ',,,IPv4,10.0.0.1,Privée,,10.0.0.2,Privée,,1234,80,TCP,HTTP,2,120,2026-07-03 15:09:40.123456 UTC,,'
matrix_seed null.csv NULL \
  ',,,IPv6,fe80::1,Lien local,,ff02::1,Multicast,,,,ICMPv6,,1,86,2026-07-03 15:09:40 UTC,,'
matrix_seed sll.csv LINUX_SLL \
  '01:02:03:04,,,0xABCD,10.0.0.1,Privée,,10.0.0.2,Privée,,1234,80,TCP,,2,120,2026-07-03 15:09:40.123456 UTC,,'
matrix_seed sll2.csv LINUX_SLL2 \
  '01:02:03:04:05:06:07:08,,,IPv4,10.0.0.1,Privée,,10.0.0.2,Privée,,53,53,UDP,DNS,2,120,2026-07-03 15:09:40.123456 UTC,,'

# Préambule coupé en plein échappement.
printf '#LFMS version=2 dlt=ETHERNET site=Usine%%2' >"$matrix_corpus/preambule_coupe.csv"

# --- winaudit_reader --------------------------------------------------------
# Relevés versionnés (UTF-16LE avec BOM), puis le premier réencodé dans les
# autres encodages acceptés : UTF-8 avec et sans BOM, UTF-16BE.
survey="$fixtures/winaudit/20260924_NP_CONF_TST-PC-000001.csv"
cp -- "$survey" "$winaudit_corpus/"
find "$fixtures/winaudit/parite-winaudit2labels/releves" -type f -name '*.csv' \
  -exec cp -- {} "$winaudit_corpus/" \;
iconv -f UTF-16 -t UTF-8 "$survey" >"$winaudit_corpus/utf8.csv"
{ printf '\xEF\xBB\xBF'; cat "$winaudit_corpus/utf8.csv"; } >"$winaudit_corpus/utf8_bom.csv"
{ printf '\xFE\xFF'; iconv -f UTF-16 -t UTF-16BE "$survey"; } >"$winaudit_corpus/utf16be.csv"

# --- xlsx_reader ------------------------------------------------------------
# Chaque classeur versionné donne deux graines : le fichier tel quel (octet
# de forme 0) et le XML de sa première feuille, que la cible emballe
# elle-même (octet de forme 1).
for workbook in "$fixtures"/*.xlsx; do
  name="$(basename -- "$workbook" .xlsx)"
  { printf '\000'; cat -- "$workbook"; } >"$xlsx_corpus/$name.classeur"
  { printf '\001'; unzip -p -- "$workbook" xl/worksheets/sheet1.xml; } >"$xlsx_corpus/$name.feuille"
done
# Feuille à chaînes partagées, cellules numériques, date et erreur.
{
  printf '\001'
  printf '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>'
  printf '<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" t="s"><v>2</v></c></row>'
  printf '<row r="3"><c r="A3"><v>1122334455</v></c><c r="B3" t="b"><v>1</v></c><c r="C3" t="e"><v>#N/A</v></c></row>'
  printf '</sheetData></worksheet>'
  printf '\n<!--LUNAR-SHARED-STRINGS-->\n'
  printf '<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>mac</t></si><si><t>ip</t></si><si><r><t>la</t></r><r><t>bel</t></r></si></sst>'
} >"$xlsx_corpus/chaines_partagees.feuille"

# --- Graines de régression --------------------------------------------------
# Chaque crash corrigé reste dans le corpus : regressions/<cible>/ est
# versionné et recopié ici.
for target in pcap_reader matrix_reader winaudit_reader packet_frames xlsx_reader; do
  if compgen -G "$fuzz/regressions/$target/*" >/dev/null; then
    cp -- "$fuzz/regressions/$target"/* "$fuzz/corpus/$target/"
  fi
done

for target in pcap_reader matrix_reader winaudit_reader packet_frames xlsx_reader; do
  printf 'Corpus %s : %s graines\n' "$target" \
    "$(find "$fuzz/corpus/$target" -maxdepth 1 -type f | wc -l)"
done
