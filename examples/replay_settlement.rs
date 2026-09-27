//! Offline acceptance harness. Only RPC payload bytes and explicit catalog
//! relations are inputs; CombatEvidence and decoded numeric fields are not.
use anyhow::{Context, Result, bail};
use nte_dps_tool::engine::settlement::transport::{Decoder, Profile, Rpc};
use nte_dps_tool::engine::settlement::{
    Ledger, MessageKey, Skill, SkillCatalog, decode_request, decode_settlement,
};
use pcap_file::pcapng::{Block, PcapNgReader};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, fs, net::Ipv4Addr, path::Path};

#[derive(Deserialize)]
struct CaptureProfile {
    rpc: Profile,
    local_ip: Ipv4Addr,
    local_port: u16,
    server_ip: Ipv4Addr,
    server_port: u16,
}

fn pcap(
    path: &Path,
    profile: CaptureProfile,
    mut accept: impl FnMut(Rpc) -> Result<()>,
) -> Result<usize> {
    nte_dps_tool::engine::capture::validate_pcapng_import(path)?;
    if fs::metadata(path)?.len() > 512 * 1024 * 1024 {
        bail!("capture_budget_exceeded");
    }
    let mut reader = PcapNgReader::new(fs::File::open(path)?)?;
    let mut decoder = Decoder::new(profile.rpc).map_err(|e| anyhow::anyhow!("profile: {e:?}"))?;
    let mut packets = 0usize;
    let mut blocks = 0usize;
    let mut interfaces = Vec::new();
    while let Some(block) = reader.next_block() {
        blocks += 1;
        if blocks > 2_000_000 {
            bail!("block_budget_exceeded");
        }
        match block? {
            Block::SectionHeader(_) => {
                interfaces.clear();
                decoder.clear();
            }
            Block::InterfaceDescription(i) => interfaces.push(i.linktype),
            Block::EnhancedPacket(epb) => {
                if interfaces.get(epb.interface_id as usize) != Some(&pcap_file::DataLink::ETHERNET)
                {
                    bail!("unsupported_capture_link_type");
                }
                let data = epb.data.as_ref();
                if data.len() < 34 || data[12..14] != [8, 0] || data[23] != 17 {
                    continue;
                }
                let ip = &data[14..];
                if ip[0] >> 4 != 4 {
                    bail!("invalid_ipv4");
                }
                let ihl = usize::from(ip[0] & 15) * 4;
                if ihl < 20 || ip.len() < ihl + 8 {
                    bail!("truncated_ipv4_udp");
                }
                let source = Ipv4Addr::new(ip[12], ip[13], ip[14], ip[15]);
                let dest = Ipv4Addr::new(ip[16], ip[17], ip[18], ip[19]);
                let udp = &ip[ihl..];
                let sport = u16::from_be_bytes([udp[0], udp[1]]);
                let dport = u16::from_be_bytes([udp[2], udp[3]]);
                let inbound = source == profile.server_ip
                    && sport == profile.server_port
                    && dest == profile.local_ip
                    && dport == profile.local_port;
                let outbound = dest == profile.server_ip
                    && dport == profile.server_port
                    && source == profile.local_ip
                    && sport == profile.local_port;
                if !inbound && !outbound {
                    continue;
                }
                if u16::from_be_bytes([ip[6], ip[7]]) & 0x3fff != 0 {
                    bail!("fragmented_ip_not_supported");
                }
                let total = usize::from(u16::from_be_bytes([ip[2], ip[3]]));
                let len = usize::from(u16::from_be_bytes([udp[4], udp[5]]));
                if total > ip.len() || len < 8 || ihl + len != total {
                    bail!("udp_length_mismatch");
                }
                packets += 1;
                let messages = decoder
                    .datagram(&udp[8..len], inbound)
                    .map_err(|e| anyhow::anyhow!("datagram {packets}: {e:?}"))?;
                for message in messages {
                    accept(message)?;
                }
            }
            _ => {}
        }
    }
    if decoder.has_incomplete_fragments() {
        bail!("incomplete_final_fragment");
    }
    Ok(packets)
}

fn read(path: &Path) -> Result<Value> {
    if fs::metadata(path)?.len() > 128 * 1024 * 1024 {
        bail!("input_budget_exceeded");
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 && args.len() != 4 {
        bail!(
            "usage: replay_settlement RPC_JSON CATALOG_JSON OUTPUT_JSON | PCAPNG PROFILE_JSON CATALOG_JSON OUTPUT_JSON"
        );
    }
    let capture_mode = args.len() == 4;
    let catalog = read(Path::new(&args[args.len() - 2]))?;
    let mut skills = SkillCatalog::default();
    for effect in catalog["damageEffects"]
        .as_object()
        .context("damageEffects")?
        .values()
    {
        let Some(index) = effect["effectIndex"]
            .as_u64()
            .and_then(|i| u32::try_from(i).ok())
        else {
            continue;
        };
        let Some(root) = effect["statisticalSkillKey"].as_str() else {
            skills.unresolved_effects.insert(index);
            continue;
        };
        let skill = &catalog["skills"][root];
        let owners = skill["owners"]
            .as_array()
            .context("owners")?
            .iter()
            .filter_map(|o| o["characterId"].as_str()?.parse().ok())
            .collect();
        skills.effects.entry(index).or_default().push(Skill {
            key: root.to_owned(),
            name: skill["name"].as_str().map(str::to_owned),
            owners,
        });
    }
    let mut ledger = Ledger::new(100_000, skills);
    let mut projections = HashMap::<MessageKey, _>::new();
    let mut counts = [0usize; 2];
    let mut accept = |message| -> Result<()> {
        let change = match message {
            Rpc::Inventory { .. } => return Ok(()),
            Rpc::UnsupportedSettlementExtras => bail!("unsupported_settlement_extras"),
            Rpc::Request(r) => {
                counts[0] += 1;
                ledger.request(r)
            }
            Rpc::Settlement(r) => {
                counts[1] += 1;
                ledger.settlement(r)
            }
        }
        .map_err(|e| anyhow::anyhow!("ledger: {e:?}"))?;
        if let Some(c) = change {
            projections.insert(c.key, c);
        }
        Ok(())
    };
    let udp_packets = if capture_mode {
        let profile = serde_json::from_value(read(Path::new(&args[1]))?)?;
        Some(pcap(Path::new(&args[0]), profile, &mut accept)?)
    } else {
        let packet = read(Path::new(&args[0]))?;
        for record in packet["records"].as_array().context("records")? {
            let name = record["name"].as_str().context("rpc_name")?;
            let request = name == "Function HTGame.HTPlayerController._SRFD_B_Params_Array_";
            if !request
                && name != "Function HTGame.HTPlayerController.ClientSetReplicatedTargetData"
            {
                continue;
            }
            let raw = hex::decode(record["payloadHex"].as_str().context("rpc_bytes")?)?;
            let bits = usize::try_from(record["rpcBits"].as_u64().context("rpc_bits")?)?;
            let channel = u32::try_from(record["channel"].as_u64().context("channel")?)?;
            if request {
                let decoded = decode_request(&raw, bits, channel)
                    .map_err(|e| anyhow::anyhow!("request decode: {e:?}"))?;
                accept(Rpc::Request(decoded))?;
            } else {
                let decoded = decode_settlement(&raw, bits, channel)
                    .map_err(|e| anyhow::anyhow!("settlement decode: {e:?}"))?;
                accept(Rpc::Settlement(decoded))?;
            }
        }
        None
    };
    let mut changes: Vec<_> = projections.into_values().collect();
    changes.sort_by_key(|c| (c.key.channel, c.key.message, c.key.timestamp_bits));
    let rows: Vec<_> = changes.iter().flat_map(|c| &c.rows).collect();
    let total: i64 = rows.iter().map(|r| i64::from(r.damage)).sum();
    let output = serde_json::json!({"schema":"nte.rust.settlement_replay/1", "requests":counts[0], "settlements":counts[1],
        "damageComponents":rows.len(), "totalDamageAllDirections":total,"changes":changes,"udpPackets":udp_packets,
        "nativeEvidenceInputUsed":false,"inputScope":"explicitly qualified RPC boundaries; not automatic PCAP class binding",
        "productionPromotionAllowed":false});
    let bytes = serde_json::to_vec_pretty(&output)?;
    let output_path = Path::new(&args[args.len() - 1]);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_path)?;
    std::io::Write::write_all(&mut file, &bytes)?;
    anyhow::ensure!(read(output_path)? == output, "output_readback_mismatch");
    println!(
        "requests={} settlements={} components={} damage_all_directions={total}",
        counts[0],
        counts[1],
        rows.len()
    );
    Ok(())
}
