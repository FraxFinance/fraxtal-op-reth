use std::sync::Arc;

use crate::network::FraxtalNetworkBuilder;
use fraxtal_evm::FraxtalEvmConfig;
use reth_chainspec::{BaseFeeParams, EthereumHardforks};
use reth_node_api::{FullNodeComponents, PayloadAttributesBuilder, PayloadTypes};
use reth_node_builder::{
    BuilderContext, DebugNode, Node, NodeAdapter, NodeComponentsBuilder,
    components::{BasicPayloadServiceBuilder, ComponentsBuilder, ExecutorBuilder},
    node::{FullNodeTypes, NodeTypes},
    rpc::BasicEngineValidatorBuilder,
};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::OpRethReceiptBuilder;
use reth_optimism_forks::OpHardforks;
use reth_optimism_node::{
    OpAddOnsBuilder, OpEngineApiBuilder, OpEngineTypes, OpFullNodeTypes, OpStorage,
    args::RollupArgs,
    node::{
        OpAddOns, OpConsensusBuilder, OpEngineValidatorBuilder, OpNodeTypes, OpPayloadBuilder,
        OpPoolBuilder,
    },
    txpool::interop::InteropFailsafe,
};
use reth_optimism_payload_builder::{
    OpPayloadAttrs,
    config::{OpBuilderConfig, OpDAConfig, OpGasLimitConfig, OperatorSdmOptIn},
};
use reth_optimism_primitives::OpPrimitives;
use reth_optimism_rpc::eth::OpEthApiBuilder;
use reth_provider::providers::ProviderFactoryBuilder;
use reth_rpc_api::eth::RpcTypes;

/// Builds [`OpPayloadAttrs`] for local/dev-mode payload generation.
struct OpLocalPayloadAttributesBuilder {
    chain_spec: Arc<OpChainSpec>,
}

impl PayloadAttributesBuilder<OpPayloadAttrs> for OpLocalPayloadAttributesBuilder {
    fn build(
        &self,
        parent: &reth_primitives_traits::SealedHeader<alloy_consensus::Header>,
    ) -> OpPayloadAttrs {
        use alloy_consensus::BlockHeader;
        use alloy_primitives::{Address, B64};

        let timestamp = std::cmp::max(
            parent.timestamp().saturating_add(1),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
        );

        let eth_attrs = alloy_rpc_types_engine::PayloadAttributes {
            timestamp,
            prev_randao: alloy_primitives::B256::random(),
            suggested_fee_recipient: Address::random(),
            withdrawals: self
                .chain_spec
                .is_shanghai_active_at_timestamp(timestamp)
                .then(Default::default),
            parent_beacon_block_root: self
                .chain_spec
                .is_cancun_active_at_timestamp(timestamp)
                .then(alloy_primitives::B256::random),
            slot_number: None,
            target_gas_limit: None,
        };

        /// Dummy system transaction for dev mode.
        /// OP Mainnet transaction at index 0 in block 124665056.
        const TX_SET_L1_BLOCK: [u8; 251] = alloy_primitives::hex!(
            "7ef8f8a0683079df94aa5b9cf86687d739a60a9b4f0835e520ec4d664e2e415dca17a6df94deaddeaddeaddeaddeaddeaddeaddeaddead00019442000000000000000000000000000000000000158080830f424080b8a4440a5e200000146b000f79c500000000000000040000000066d052e700000000013ad8a3000000000000000000000000000000000000000000000000000000003ef1278700000000000000000000000000000000000000000000000000000000000000012fdf87b89884a61e74b322bbcf60386f543bfae7827725efaaf0ab1de2294a590000000000000000000000006887246668a3b87f54deb3b94ba47a6f63f32985"
        );

        let default_params = BaseFeeParams::optimism();
        let denominator = std::env::var("OP_DEV_EIP1559_DENOMINATOR")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(default_params.max_change_denominator as u32);
        let elasticity = std::env::var("OP_DEV_EIP1559_ELASTICITY")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(default_params.elasticity_multiplier as u32);
        let gas_limit = std::env::var("OP_DEV_GAS_LIMIT").ok().and_then(|v| v.parse::<u64>().ok());

        let mut eip1559_bytes = [0u8; 8];
        eip1559_bytes[0..4].copy_from_slice(&denominator.to_be_bytes());
        eip1559_bytes[4..8].copy_from_slice(&elasticity.to_be_bytes());

        OpPayloadAttrs(op_alloy_rpc_types_engine::OpPayloadAttributes {
            payload_attributes: eth_attrs,
            transactions: Some(vec![TX_SET_L1_BLOCK.into()]),
            no_tx_pool: None,
            gas_limit,
            eip_1559_params: Some(B64::from(eip1559_bytes)),
            min_base_fee: Some(0),
        })
    }
}

/// Type configuration for a regular Optimism node.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FraxtalNode {
    /// Additional Optimism args
    pub args: RollupArgs,
    /// Data availability configuration for the OP builder.
    ///
    /// Used to throttle the size of the data availability payloads (configured by the batcher via
    /// the `miner_` api).
    ///
    /// By default no throttling is applied.
    pub da_config: OpDAConfig,
    /// Gas limit configuration for the OP builder.
    /// Used to control the gas limit of the blocks produced by the OP builder.(configured by the
    /// batcher via the `miner_` api)
    pub gas_limit_config: OpGasLimitConfig,
    /// Local operator opt-in for SDM `PostExec` production. Shared (via Arc clones) between the
    /// payload builder and the `admin_setOperatorSdmOptIn` RPC handler.
    pub operator_sdm_opt_in: OperatorSdmOptIn,
    /// Interop failsafe gate, shared between the txpool's interop filter client (writer) and the
    /// payload builder (reader, to exclude interop txs while it is active).
    pub interop_failsafe: InteropFailsafe,
}

/// A [`ComponentsBuilder`] with its generic arguments set to a stack of Optimism specific builders.
pub type FraxtalNodeComponentBuilder<Node, Payload = OpPayloadBuilder> = ComponentsBuilder<
    Node,
    OpPoolBuilder,
    BasicPayloadServiceBuilder<Payload>,
    FraxtalNetworkBuilder,
    FraxtalExecutorBuilder,
    OpConsensusBuilder,
>;

impl Default for FraxtalNode {
    fn default() -> Self {
        Self::new(RollupArgs::default())
    }
}

impl FraxtalNode {
    /// Creates a new instance of the Optimism node type.
    pub fn new(args: RollupArgs) -> Self {
        let operator_sdm_opt_in = OperatorSdmOptIn::configured(args.operator_sdm_opt_in);
        Self {
            args,
            da_config: OpDAConfig::default(),
            gas_limit_config: OpGasLimitConfig::default(),
            operator_sdm_opt_in,
            interop_failsafe: InteropFailsafe::default(),
        }
    }

    /// Configure the data availability configuration for the OP builder.
    pub fn with_da_config(mut self, da_config: OpDAConfig) -> Self {
        self.da_config = da_config;
        self
    }

    /// Configure the gas limit configuration for the OP builder.
    pub fn with_gas_limit_config(mut self, gas_limit_config: OpGasLimitConfig) -> Self {
        self.gas_limit_config = gas_limit_config;
        self
    }

    /// The [`OpBuilderConfig`] this node's payload builder is configured with.
    ///
    /// This is the single assembly site for the payload builder's config: [`components`] and any
    /// downstream node that replaces the payload component consume it wholesale, so they stay in
    /// lockstep as [`OpBuilderConfig`] gains fields. Because [`OpBuilderConfig`] is not
    /// `#[non_exhaustive]`, adding a field forces a compile error here until it is threaded
    /// through.
    ///
    /// [`components`]: FraxtalNode::components
    pub fn builder_config(&self) -> OpBuilderConfig {
        OpBuilderConfig {
            da_config: self.da_config.clone(),
            gas_limit_config: self.gas_limit_config.clone(),
            operator_sdm_opt_in: self.operator_sdm_opt_in.clone(),
            interop_failsafe: self.interop_failsafe.clone(),
            max_uncompressed_block_size: self.args.max_uncompressed_block_size,
        }
    }

    /// The pool builder configured the way every OP node's pool expects: tx-conditional support,
    /// interop endpoints and quorum, and the shared interop failsafe gate.
    ///
    /// Generic over the pooled-transaction type so a downstream node can start from this base and
    /// chain [`with_ordering`](OpPoolBuilder::with_ordering) /
    /// [`with_validator_wrapper`](OpPoolBuilder::with_validator_wrapper) to swap in a custom `T`,
    /// ordering, or validator wrapper.
    pub fn standard_pool_builder<T>(&self) -> OpPoolBuilder<T> {
        OpPoolBuilder::<T>::default()
            .with_enable_tx_conditional(self.args.enable_tx_conditional)
            .with_interop(
                self.args.interop_http.clone(),
                self.args.interop_min_responses,
                self.args.interop_safety_level,
            )
            .with_interop_failsafe(self.interop_failsafe.clone())
    }

    /// Returns the payload builder stock components install.
    ///
    /// Exposed so a wrapper node that replaces only the payload *service* still builds payloads
    /// from the same configured builder.
    pub fn payload_builder(&self) -> OpPayloadBuilder {
        OpPayloadBuilder::new(self.args.compute_pending_block)
            .with_builder_config(self.builder_config())
    }

    /// Returns the components for the given [`RollupArgs`].
    pub fn components<Node>(&self) -> FraxtalNodeComponentBuilder<Node>
    where
        Node: FullNodeTypes<Types: OpNodeTypes>,
    {
        let RollupArgs { disable_txpool_gossip, discovery_v4, .. } = self.args;
        ComponentsBuilder::default()
            .node_types::<Node>()
            .executor(FraxtalExecutorBuilder::default())
            .pool(self.standard_pool_builder())
            .payload(BasicPayloadServiceBuilder::new(self.payload_builder()))
            .network(FraxtalNetworkBuilder::new(disable_txpool_gossip, !discovery_v4))
            .consensus(OpConsensusBuilder::default())
    }

    /// Returns [`OpAddOnsBuilder`] with configured arguments.
    pub fn add_ons_builder<NetworkT: RpcTypes>(&self) -> OpAddOnsBuilder<NetworkT> {
        OpAddOnsBuilder::default()
            .with_sequencer(self.args.sequencer.clone())
            .with_sequencer_headers(self.args.sequencer_headers.clone())
            .with_da_config(self.da_config.clone())
            .with_gas_limit_config(self.gas_limit_config.clone())
            .with_operator_sdm_opt_in(self.operator_sdm_opt_in.clone())
            .with_enable_tx_conditional(self.args.enable_tx_conditional)
            .with_min_suggested_priority_fee(self.args.min_suggested_priority_fee)
            .with_historical_rpc(self.args.historical_rpc.clone())
            .with_flashblocks(self.args.flashblocks_url.clone())
            .with_flashblock_consensus(self.args.flashblock_consensus)
            .with_retain_forwarded_txs(self.args.retain_forwarded_txs)
    }

    /// Instantiates the [`ProviderFactoryBuilder`] for an opstack node.
    ///
    /// # Open a Providerfactory in read-only mode from a datadir
    ///
    /// See also: [`ProviderFactoryBuilder`] and
    /// [`ReadOnlyConfig`](reth_provider::providers::ReadOnlyConfig).
    ///
    /// ```no_run
    /// use reth_optimism_chainspec::OP_MAINNET;
    /// use reth_optimism_node::OpNode;
    ///
    /// fn demo(runtime: reth_tasks::Runtime) {
    ///     let factory = OpNode::provider_factory_builder()
    ///         .open_read_only(OP_MAINNET.clone(), "datadir", runtime)
    ///         .unwrap();
    /// }
    /// ```
    ///
    /// # Open a Providerfactory with custom config
    ///
    /// ```no_run
    /// use reth_optimism_chainspec::OpChainSpecBuilder;
    /// use reth_optimism_node::OpNode;
    /// use reth_provider::providers::ReadOnlyConfig;
    ///
    /// fn demo(runtime: reth_tasks::Runtime) {
    ///     let factory = OpNode::provider_factory_builder()
    ///         .open_read_only(
    ///             OpChainSpecBuilder::optimism_mainnet().build().into(),
    ///             ReadOnlyConfig::from_datadir("datadir").no_watch(),
    ///             runtime,
    ///         )
    ///         .unwrap();
    /// }
    /// ```
    pub fn provider_factory_builder() -> ProviderFactoryBuilder<Self> {
        ProviderFactoryBuilder::default()
    }
}

impl<N> Node<N> for FraxtalNode
where
    N: FullNodeTypes<Types: OpFullNodeTypes + OpNodeTypes>,
{
    type ComponentsBuilder = ComponentsBuilder<
        N,
        OpPoolBuilder,
        BasicPayloadServiceBuilder<OpPayloadBuilder>,
        FraxtalNetworkBuilder,
        FraxtalExecutorBuilder,
        OpConsensusBuilder,
    >;

    type AddOns = OpAddOns<
        NodeAdapter<N, <Self::ComponentsBuilder as NodeComponentsBuilder<N>>::Components>,
        OpEthApiBuilder,
        OpEngineValidatorBuilder,
        OpEngineApiBuilder<OpEngineValidatorBuilder>,
        BasicEngineValidatorBuilder<OpEngineValidatorBuilder>,
    >;

    fn components_builder(&self) -> Self::ComponentsBuilder {
        Self::components(self)
    }

    fn add_ons(&self) -> Self::AddOns {
        self.add_ons_builder().build()
    }
}

impl<N> DebugNode<N> for FraxtalNode
where
    N: FullNodeComponents<Types = Self>,
{
    type RpcBlock = alloy_rpc_types_eth::Block<op_alloy_consensus::OpTxEnvelope>;

    fn rpc_to_primitive_block(rpc_block: Self::RpcBlock) -> reth_node_api::BlockTy<Self> {
        rpc_block.into_consensus()
    }

    fn local_payload_attributes_builder(
        chain_spec: &Self::ChainSpec,
    ) -> impl PayloadAttributesBuilder<<Self::Payload as PayloadTypes>::PayloadAttributes> {
        OpLocalPayloadAttributesBuilder { chain_spec: Arc::new(chain_spec.clone()) }
    }
}

impl NodeTypes for FraxtalNode {
    type Primitives = OpPrimitives;
    type ChainSpec = OpChainSpec;
    type Storage = OpStorage;
    type Payload = OpEngineTypes;
}

/// A regular optimism evm and executor builder.
#[derive(Debug, Copy, Clone, Default)]
#[non_exhaustive]
pub struct FraxtalExecutorBuilder {}

impl<Node> ExecutorBuilder<Node> for FraxtalExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec: OpHardforks, Primitives = OpPrimitives>>,
{
    type EVM = FraxtalEvmConfig<
        <Node::Types as NodeTypes>::ChainSpec,
        <Node::Types as NodeTypes>::Primitives,
    >;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> eyre::Result<Self::EVM> {
        Ok(FraxtalEvmConfig::new(ctx.chain_spec(), OpRethReceiptBuilder::default()))
    }
}
