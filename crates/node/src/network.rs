//! Network builder that injects Fraxtal-specific bootnodes.
//!
//! Mirrors `reth_optimism_node::node::OpNetworkBuilder` but overrides the fallback bootnodes
//! (`mainnet_nodes()` in upstream reth) with Fraxtal's bootnodes for known chain ids.
//! The user-supplied `--bootnodes` CLI flag still takes precedence.

use fraxtal_chainspec::fraxtal_bootnodes;
use reth_chainspec::{EthChainSpec, Hardforks};
use reth_network::{
    NetworkConfig, NetworkHandle, NetworkManager, NetworkPrimitives, PeersInfo,
    types::BasicNetworkPrimitives,
};
use reth_node_api::{PrimitivesTy, TxTy};
use reth_node_builder::{
    BuilderContext,
    components::NetworkBuilder,
    node::{FullNodeTypes, NodeTypes},
};
use reth_transaction_pool::{PoolPooledTx, PoolTransaction, TransactionPool};
use tracing::info;

/// Network builder for Fraxtal nodes.
#[derive(Debug, Default, Clone)]
pub struct FraxtalNetworkBuilder {
    /// Disable transaction pool gossip.
    pub disable_txpool_gossip: bool,
    /// Disable discovery v4.
    pub disable_discovery_v4: bool,
}

impl FraxtalNetworkBuilder {
    /// Creates a new `FraxtalNetworkBuilder`.
    pub const fn new(disable_txpool_gossip: bool, disable_discovery_v4: bool) -> Self {
        Self { disable_txpool_gossip, disable_discovery_v4 }
    }

    /// Builds the [`NetworkConfig`] for the node, injecting Fraxtal bootnodes for known chains.
    pub fn network_config<Node, NetworkP>(
        &self,
        ctx: &BuilderContext<Node>,
    ) -> eyre::Result<NetworkConfig<Node::Provider, NetworkP>>
    where
        Node: FullNodeTypes<Types: NodeTypes<ChainSpec: Hardforks + EthChainSpec>>,
        NetworkP: NetworkPrimitives,
    {
        let disable_txpool_gossip = self.disable_txpool_gossip;
        let disable_discovery_v4 = self.disable_discovery_v4;
        let args = &ctx.config().network;

        let chain_id = ctx.chain_spec().chain().id();
        // Respect --bootnodes when set; otherwise fall back to Fraxtal's built-in list
        // (upstream reth would fall back to Ethereum mainnet bootnodes here).
        let fraxtal_nodes = args.bootnodes.is_none().then(|| fraxtal_bootnodes(chain_id)).flatten();

        let mut builder = ctx.network_config_builder()?;
        if let Some(nodes) = &fraxtal_nodes {
            info!(
                target: "reth::cli",
                count = nodes.len(),
                chain_id,
                "Using Fraxtal bootnodes",
            );
            builder = builder.boot_nodes(nodes.clone());
        }

        let network_builder = builder.apply(|mut builder| {
            let rlpx_socket = (args.addr, args.port).into();
            if disable_discovery_v4 || args.discovery.disable_discovery {
                builder = builder.disable_discv4_discovery();
            }
            if !args.discovery.disable_discovery {
                let discv5_bootnodes = args
                    .resolved_bootnodes()
                    .or(fraxtal_nodes)
                    .or_else(|| ctx.chain_spec().bootnodes())
                    .unwrap_or_default();

                let mut discv5_builder =
                    args.discovery.discovery_v5_builder(rlpx_socket, discv5_bootnodes);

                // Workaround for https://github.com/paradigmxyz/reth/pull/23639 (open
                // upstream): when the listen address is unspecified (0.0.0.0) and the
                // user provides --nat extip:<IP>, reth's `build_local_enr` skips
                // setting the ENR `ip` field, leaving the node undiscoverable over
                // discv5. Inject the resolved external IPv4 directly as the standard
                // ENR `ip` kv pair (RLP-encoded octets, matching `enr::Builder::ip4`).
                if args.addr.is_unspecified() &&
                    let Some(std::net::IpAddr::V4(ip)) = args.nat.clone().as_external_ip(0) &&
                    !ip.is_unspecified()
                {
                    info!(
                        target: "reth::cli",
                        %ip,
                        "Injecting NAT external IPv4 into discv5 ENR (listen addr is unspecified)",
                    );
                    discv5_builder = discv5_builder.add_enr_kv_pair(
                        b"ip",
                        alloy_primitives::Bytes::from(alloy_rlp::encode(ip.octets().as_slice())),
                    );
                }

                builder = builder.discovery_v5(discv5_builder);
            }
            builder
        });

        let mut network_config = ctx.build_network_config(network_builder);
        network_config.tx_gossip_disabled = disable_txpool_gossip;

        Ok(network_config)
    }
}

impl<Node, Pool> NetworkBuilder<Node, Pool> for FraxtalNetworkBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec: Hardforks + EthChainSpec>>,
    Pool: TransactionPool<Transaction: PoolTransaction<Consensus = TxTy<Node::Types>>>
        + Unpin
        + 'static,
{
    type Network =
        NetworkHandle<BasicNetworkPrimitives<PrimitivesTy<Node::Types>, PoolPooledTx<Pool>>>;

    async fn build_network(
        self,
        ctx: &BuilderContext<Node>,
        pool: Pool,
    ) -> eyre::Result<Self::Network> {
        let network_config = self.network_config(ctx)?;
        let network = NetworkManager::builder(network_config).await?;
        let handle = ctx.start_network(network, pool);
        info!(target: "reth::cli", enode = %handle.local_node_record(), "P2P networking initialized");
        Ok(handle)
    }
}
