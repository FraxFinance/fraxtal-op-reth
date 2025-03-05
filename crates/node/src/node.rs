use fraxtal_evm::execute::FraxtalExecutionStrategyFactory;
use reth_evm::{execute::BasicBlockExecutorProvider, ConfigureEvmFor};
use reth_node_api::{PrimitivesTy, TxTy};
use reth_node_builder::{
    components::{ComponentsBuilder, ExecutorBuilder, PayloadServiceBuilder},
    BuilderContext, FullNodeTypes, Node, NodeAdapter, NodeComponentsBuilder, NodeTypes,
    NodeTypesWithEngine,
};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_node::{
    args::RollupArgs,
    node::{OpAddOns, OpConsensusBuilder, OpNetworkBuilder, OpPoolBuilder, OpStorage},
    BasicOpReceiptBuilder, OpEngineTypes, OpEvmConfig, OpNode,
};
use reth_optimism_payload_builder::{
    builder::OpPayloadTransactions,
    config::{OpBuilderConfig, OpDAConfig},
};
use reth_optimism_primitives::OpPrimitives;
use reth_transaction_pool::{PoolTransaction, TransactionPool};
use reth_trie_db::MerklePatriciaTrie;

#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct FraxtalNode {
    inner: OpNode,
}

impl FraxtalNode {
    pub fn new(args: RollupArgs) -> Self {
        Self {
            inner: OpNode::new(args),
        }
    }

    /// Returns the components for the given [`RollupArgs`].
    pub fn components<Node>(
        &self,
    ) -> ComponentsBuilder<
        Node,
        OpPoolBuilder,
        FraxtalPayloadBuilder,
        OpNetworkBuilder,
        FraxtalExecutorBuilder,
        OpConsensusBuilder,
    >
    where
        Node: FullNodeTypes<
            Types: NodeTypesWithEngine<
                Engine = OpEngineTypes,
                ChainSpec = OpChainSpec,
                Primitives = OpPrimitives,
            >,
        >,
    {
        let RollupArgs {
            disable_txpool_gossip,
            compute_pending_block,
            discovery_v4,
            ..
        } = self.inner.args;
        ComponentsBuilder::default()
            .node_types::<Node>()
            .pool(OpPoolBuilder::default())
            .payload(
                FraxtalPayloadBuilder::new(compute_pending_block)
                    .with_da_config(self.inner.da_config.clone()),
            )
            .network(OpNetworkBuilder {
                disable_txpool_gossip,
                disable_discovery_v4: !discovery_v4,
            })
            .executor(FraxtalExecutorBuilder::default())
            .consensus(OpConsensusBuilder::default())
    }
}

impl<N> Node<N> for FraxtalNode
where
    N: FullNodeTypes<
        Types: NodeTypesWithEngine<
            Engine = OpEngineTypes,
            ChainSpec = OpChainSpec,
            Primitives = OpPrimitives,
            Storage = OpStorage,
        >,
    >,
{
    type ComponentsBuilder = ComponentsBuilder<
        N,
        OpPoolBuilder,
        FraxtalPayloadBuilder,
        OpNetworkBuilder,
        FraxtalExecutorBuilder,
        OpConsensusBuilder,
    >;

    type AddOns =
        OpAddOns<NodeAdapter<N, <Self::ComponentsBuilder as NodeComponentsBuilder<N>>::Components>>;

    fn components_builder(&self) -> Self::ComponentsBuilder {
        Self::components(self)
    }

    fn add_ons(&self) -> Self::AddOns {
        Self::AddOns::builder()
            .with_sequencer(self.inner.args.sequencer_http.clone())
            .with_da_config(self.inner.da_config.clone())
            .build()
    }
}

impl NodeTypes for FraxtalNode {
    type Primitives = OpPrimitives;
    type ChainSpec = OpChainSpec;
    type StateCommitment = MerklePatriciaTrie;
    type Storage = OpStorage;
}

impl NodeTypesWithEngine for FraxtalNode {
    type Engine = OpEngineTypes;
}

#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct FraxtalExecutorBuilder;

impl<Node> ExecutorBuilder<Node> for FraxtalExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = OpChainSpec, Primitives = OpPrimitives>>,
{
    type EVM = OpEvmConfig;
    type Executor = BasicBlockExecutorProvider<FraxtalExecutionStrategyFactory<OpPrimitives>>;

    async fn build_evm(
        self,
        ctx: &BuilderContext<Node>,
    ) -> eyre::Result<(Self::EVM, Self::Executor)> {
        let evm_config = OpEvmConfig::new(ctx.chain_spec());
        let strategy_factory = FraxtalExecutionStrategyFactory::optimism(ctx.chain_spec());
        let executor = BasicBlockExecutorProvider::new(strategy_factory);

        Ok((evm_config, executor))
    }
}

/// Fraxtal
#[derive(Debug, Default, Clone)]
pub struct FraxtalPayloadBuilder<Txs = ()> {
    /// By default the pending block equals the latest block
    /// to save resources and not leak txs from the tx-pool,
    /// this flag enables computing of the pending block
    /// from the tx-pool instead.
    ///
    /// If `compute_pending_block` is not enabled, the payload builder
    /// will use the payload attributes from the latest block. Note
    /// that this flag is not yet functional.
    pub compute_pending_block: bool,
    /// The type responsible for yielding the best transactions for the payload if mempool
    /// transactions are allowed.
    pub best_transactions: Txs,
    /// This data availability configuration specifies constraints for the payload builder
    /// when assembling payloads
    pub da_config: OpDAConfig,
}

impl FraxtalPayloadBuilder {
    /// Create a new instance with the given `compute_pending_block` flag and data availability
    /// config.
    pub fn new(compute_pending_block: bool) -> Self {
        Self {
            compute_pending_block,
            best_transactions: (),
            da_config: OpDAConfig::default(),
        }
    }

    /// Configure the data availability configuration for the OP payload builder.
    pub fn with_da_config(mut self, da_config: OpDAConfig) -> Self {
        self.da_config = da_config;
        self
    }
}

impl<Txs> FraxtalPayloadBuilder<Txs> {
    /// Configures the type responsible for yielding the transactions that should be included in the
    /// payload.
    pub fn with_transactions<T>(self, best_transactions: T) -> FraxtalPayloadBuilder<T> {
        let Self {
            compute_pending_block,
            da_config,
            ..
        } = self;
        FraxtalPayloadBuilder {
            compute_pending_block,
            best_transactions,
            da_config,
        }
    }

    /// A helper method to initialize [`reth_optimism_payload_builder::OpPayloadBuilder`] with the
    /// given EVM config.
    #[expect(clippy::type_complexity)]
    pub fn build<Node, Evm, Pool>(
        &self,
        evm_config: Evm,
        ctx: &BuilderContext<Node>,
        pool: Pool,
    ) -> eyre::Result<
        fraxtal_payload_builder::builder::FraxtalPayloadBuilder<
            Pool,
            Node::Provider,
            Evm,
            PrimitivesTy<Node::Types>,
            Txs,
        >,
    >
    where
        Node: FullNodeTypes<
            Types: NodeTypesWithEngine<
                Engine = OpEngineTypes,
                ChainSpec = OpChainSpec,
                Primitives = OpPrimitives,
            >,
        >,
        Pool: TransactionPool<Transaction: PoolTransaction<Consensus = TxTy<Node::Types>>>
            + Unpin
            + 'static,
        Evm: ConfigureEvmFor<PrimitivesTy<Node::Types>>,
        Txs: OpPayloadTransactions<Pool::Transaction>,
    {
        let payload_builder =
            fraxtal_payload_builder::builder::FraxtalPayloadBuilder::with_builder_config(
                pool,
                ctx.provider().clone(),
                evm_config,
                BasicOpReceiptBuilder::default(),
                OpBuilderConfig {
                    da_config: self.da_config.clone(),
                },
            )
            .with_transactions(self.best_transactions.clone())
            .set_compute_pending_block(self.compute_pending_block);
        Ok(payload_builder)
    }
}

impl<Node, Pool, Txs> PayloadServiceBuilder<Node, Pool> for FraxtalPayloadBuilder<Txs>
where
    Node: FullNodeTypes<
        Types: NodeTypesWithEngine<
            Engine = OpEngineTypes,
            ChainSpec = OpChainSpec,
            Primitives = OpPrimitives,
        >,
    >,
    Pool: TransactionPool<Transaction: PoolTransaction<Consensus = TxTy<Node::Types>>>
        + Unpin
        + 'static,
    Txs: OpPayloadTransactions<Pool::Transaction>,
{
    type PayloadBuilder = fraxtal_payload_builder::builder::FraxtalPayloadBuilder<
        Pool,
        Node::Provider,
        OpEvmConfig,
        PrimitivesTy<Node::Types>,
        Txs,
    >;

    async fn build_payload_builder(
        &self,
        ctx: &BuilderContext<Node>,
        pool: Pool,
    ) -> eyre::Result<Self::PayloadBuilder> {
        self.build(OpEvmConfig::new(ctx.chain_spec()), ctx, pool)
    }
}
