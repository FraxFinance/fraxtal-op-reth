//! Discovery-only bootnode for Fraxtal.
//!
//! Forked from upstream `reth_cli_commands::p2p::bootnode` and patched to inject the NAT-resolved
//! external IPv4 into the discv5 ENR when the listen address is unspecified (0.0.0.0). This is the
//! same workaround applied to the Fraxtal full node in `fraxtal-node`'s network builder; without
//! it, peers bootstrapping from this bootnode receive an ENR with no `ip` field and cannot reach
//! it (or any peers it gossips). See https://github.com/paradigmxyz/reth/pull/23639 for the
//! upstream fix.
//!
//! Drop-in replacement for `fraxtal-op-reth p2p bootnode`.

#![allow(missing_docs, rustdoc::missing_crate_level_docs)]

use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};

use clap::Parser;
use reth_cli_util::{get_secret_key, load_secret_key::rng_secret_key};
use reth_discv4::{Discv4, Discv4Config, DiscoveryUpdate, NatResolver};
use reth_discv5::{Config, Discv5, discv5::Event};
use reth_network_peers::NodeRecord;
use secp256k1::SecretKey;
use tokio::select;
use tokio_stream::StreamExt;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

#[derive(Parser, Debug)]
#[command(name = "fraxtal-bootnode", version, about)]
struct Args {
    /// Listen address for the bootnode (UDP for discv4 / discv5, TCP advertised in ENR).
    #[arg(long, default_value = "0.0.0.0:30301")]
    addr: SocketAddr,

    /// Secret key for the bootnode (deterministic peer ID).
    /// If a path is provided but no key exists at that path, a new random secret is generated and
    /// stored there. If omitted, an ephemeral key is used.
    #[arg(long, value_name = "PATH")]
    p2p_secret_key: Option<PathBuf>,

    /// NAT resolution method (`any|none|upnp|publicip|extip:<IP>|extaddr:<host>|netif`).
    #[arg(long, default_value = "any")]
    nat: NatResolver,

    /// Also run a discv5 service on the same address.
    #[arg(long)]
    v5: bool,
}

impl Args {
    fn secret_key(&self) -> eyre::Result<SecretKey> {
        match &self.p2p_secret_key {
            Some(path) => Ok(get_secret_key(path)?),
            None => Ok(rng_secret_key()),
        }
    }
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Default to `info` when RUST_LOG is unset so operators see startup/peer events
    // without having to remember the env var.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).with_writer(std::io::stderr).init();

    let args = Args::parse();
    info!(?args, "Fraxtal bootnode starting");

    let sk = args.secret_key()?;
    let local_enr = NodeRecord::from_secret_key(args.addr, &sk);

    // discv4
    let discv4_config = Discv4Config::builder().external_ip_resolver(Some(args.nat.clone())).build();
    let (_discv4, mut discv4_service) =
        Discv4::bind(args.addr, local_enr, sk, discv4_config).await?;
    info!(?local_enr, "Started discv4");

    let mut discv4_updates = discv4_service.update_stream();
    discv4_service.spawn();

    // discv5 (optional)
    let mut discv5_updates = None;
    if args.v5 {
        let mut builder = Config::builder(args.addr);

        // Workaround for https://github.com/paradigmxyz/reth/pull/23639 (open upstream): when the
        // listen address is unspecified (0.0.0.0) and the user provides --nat extip:<IP>, reth's
        // `build_local_enr` skips setting the ENR `ip` field, leaving the bootnode undiscoverable
        // over discv5. Inject the resolved external IPv4 directly as the standard ENR `ip` kv pair
        // (RLP-encoded octets, matching `enr::Builder::ip4`).
        if args.addr.ip().is_unspecified()
            && let Some(IpAddr::V4(ip)) = args.nat.clone().as_external_ip(0)
            && !ip.is_unspecified()
        {
            info!(%ip, "Injecting NAT external IPv4 into discv5 ENR (listen addr is unspecified)");
            builder = builder.add_enr_kv_pair(
                b"ip",
                alloy_primitives::Bytes::from(alloy_rlp::encode(ip.octets().as_slice())),
            );
        }

        let (_discv5, updates) = Discv5::start(&sk, builder.build()).await?;
        info!("Started discv5");
        discv5_updates = Some(updates);
    }

    loop {
        select! {
            update = discv4_updates.next() => match update {
                Some(DiscoveryUpdate::Added(record)) => {
                    info!(peer_id = ?record.id, "(discv4) new peer added");
                }
                Some(DiscoveryUpdate::Removed(peer_id)) => {
                    info!(?peer_id, "(discv4) peer removed");
                }
                Some(_) => {}
                None => {
                    info!("(discv4) update stream ended");
                    break;
                }
            },
            update = async {
                if let Some(updates) = &mut discv5_updates {
                    updates.recv().await
                } else {
                    futures_util::future::pending().await
                }
            } => match update {
                Some(Event::SessionEstablished(enr, _)) => {
                    info!(peer_id = ?enr.id(), "(discv5) new peer added");
                }
                Some(_) => {}
                None => {
                    info!("(discv5) update stream ended");
                    break;
                }
            },
        }
    }

    Ok(())
}
