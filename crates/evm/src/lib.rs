extern crate alloc;

mod post_exec_ext;

use alloc::sync::Arc;
use alloy_consensus::{BlockHeader, Header};
use alloy_evm::{EvmFactory, FromRecoveredTx, FromTxWithEncoded, block::BlockExecutorFactory};
use alloy_op_evm::{
    OpBlockExecutionCtx, OpTx,
    block::{OpTxEnv, receipt_builder::OpReceiptBuilder},
    evm_env_for_op_block, evm_env_for_op_next_block,
};
use core::fmt::Debug;
use fraxtal_op_evm::{FraxtalBlockExecutorFactory, FraxtalEvmFactory};
use op_alloy_consensus::{
    EIP1559ParamError, OpTransaction as OpConsensusTransaction,
    parse_post_exec_payload_from_transactions,
};
use op_revm::OpSpecId;
use reth_chainspec::EthChainSpec;
use reth_evm::{ConfigureEvm, EvmEnv, eth::NextEvmEnvAttributes, precompiles::PrecompilesMap};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::{
    OpBlockAssembler, OpNextBlockEnvAttributes, OpRethReceiptBuilder, PostExecMode,
    revm_spec_by_timestamp_after_bedrock,
};
use reth_optimism_forks::OpHardforks;
use reth_optimism_primitives::{DepositReceipt, OpPrimitives};
use reth_primitives_traits::{NodePrimitives, SealedBlock, SealedHeader, SignedTransaction};
use revm::context::BlockEnv;

#[allow(unused_imports)]
use {
    alloy_eips::Decodable2718,
    alloy_primitives::{Bytes, U256},
    op_alloy_rpc_types_engine::OpExecutionData,
    reth_evm::{ConfigureEngineEvm, EvmEnvFor, ExecutableTxIterator, ExecutionCtxFor},
    reth_optimism_payload_builder::OpExecData,
    reth_primitives_traits::{TxTy, WithEncoded},
    reth_storage_errors::any::AnyError,
    revm::{
        context::CfgEnv, context_interface::block::BlobExcessGasAndPrice,
        primitives::hardfork::SpecId,
    },
};

/// Optimism-related EVM configuration.
#[derive(Debug)]
pub struct FraxtalEvmConfig<
    ChainSpec = OpChainSpec,
    N: NodePrimitives = OpPrimitives,
    R = OpRethReceiptBuilder,
    EvmFactory = FraxtalEvmFactory<OpTx>,
> {
    /// Inner [`FraxtalBlockExecutorFactory`].
    pub executor_factory: FraxtalBlockExecutorFactory<R, Arc<ChainSpec>, EvmFactory>,
    /// Optimism block assembler.
    pub block_assembler: OpBlockAssembler<ChainSpec>,
    #[doc(hidden)]
    pub _pd: core::marker::PhantomData<N>,
}

impl<ChainSpec, N: NodePrimitives, R: Clone, EvmFactory: Clone> Clone
    for FraxtalEvmConfig<ChainSpec, N, R, EvmFactory>
{
    fn clone(&self) -> Self {
        Self {
            executor_factory: self.executor_factory.clone(),
            block_assembler: self.block_assembler.clone(),
            _pd: self._pd,
        }
    }
}

impl<ChainSpec: EthChainSpec<Header = Header> + OpHardforks> FraxtalEvmConfig<ChainSpec> {
    /// Creates a new [`FraxtalEvmConfig`] with the given chain spec for OP chains.
    pub fn optimism(chain_spec: Arc<ChainSpec>) -> Self {
        Self::new(chain_spec, OpRethReceiptBuilder::default())
    }
}

impl<ChainSpec: EthChainSpec<Header = Header> + OpHardforks, N: NodePrimitives, R>
    FraxtalEvmConfig<ChainSpec, N, R>
{
    /// Creates a new [`FraxtalEvmConfig`] with the given chain spec.
    pub fn new(chain_spec: Arc<ChainSpec>, receipt_builder: R) -> Self {
        Self {
            block_assembler: OpBlockAssembler::new(chain_spec.clone()),
            executor_factory: FraxtalBlockExecutorFactory::new(
                receipt_builder,
                chain_spec,
                FraxtalEvmFactory::<OpTx>::default(),
            ),
            _pd: core::marker::PhantomData,
        }
    }
}

/// Returns true when SDM post-exec transactions are consensus-active at `timestamp`.
///
/// SDM rides the Interop hardfork: it is active iff Interop is active at `timestamp`
/// per the chain spec. This matches op-node's `IsSDM` (see `op-node/rollup/toggles.go`).
///
/// This is the single source of truth for the SDM protocol gate. Call sites that only hold a
/// chain spec should route through this helper rather than calling
/// [`OpHardforks::is_interop_active_at_timestamp`] directly, so the gate cannot drift.
pub fn is_sdm_active_at_timestamp(chain_spec: &impl OpHardforks, timestamp: u64) -> bool {
    chain_spec.is_interop_active_at_timestamp(timestamp)
}

fn post_exec_mode_from_transactions<'a, I, T>(
    transactions: I,
    block_number: u64,
    sdm_active: bool,
) -> Result<PostExecMode, EIP1559ParamError>
where
    I: IntoIterator<Item = &'a T>,
    T: OpConsensusTransaction + 'a,
{
    parse_post_exec_payload_from_transactions(transactions, block_number, sdm_active)
        .map_err(|_| EIP1559ParamError::InvalidPostExecPayload)
        .map(|parsed| {
            parsed.map_or_else(PostExecMode::default, |parsed| PostExecMode::Verify(parsed.payload))
        })
}

impl<ChainSpec, N, R, EvmFactory> FraxtalEvmConfig<ChainSpec, N, R, EvmFactory>
where
    ChainSpec: OpHardforks,
    N: NodePrimitives,
{
    /// Returns the chain spec associated with this configuration.
    pub const fn chain_spec(&self) -> &Arc<ChainSpec> {
        self.executor_factory.spec()
    }

    /// Returns true when SDM post-exec transactions are consensus-active at `timestamp`.
    ///
    /// See the free [`is_sdm_active_at_timestamp`] function, which this delegates to.
    pub fn is_sdm_active_at_timestamp(&self, timestamp: u64) -> bool {
        crate::is_sdm_active_at_timestamp(self.chain_spec(), timestamp)
    }

    /// Builds a block execution context with an optional post-exec mode override.
    pub fn context_for_block_with_post_exec_mode(
        &self,
        block: &SealedBlock<N::Block>,
        post_exec_mode: Option<PostExecMode>,
    ) -> OpBlockExecutionCtx {
        OpBlockExecutionCtx {
            parent_hash: block.header().parent_hash(),
            parent_beacon_block_root: block.header().parent_beacon_block_root(),
            extra_data: block.header().extra_data().clone(),
            post_exec_mode: post_exec_mode.unwrap_or_default(),
        }
    }

    /// Builds a next-block execution context with the provided post-exec mode.
    pub fn context_for_next_block_with_post_exec_mode(
        &self,
        parent: &SealedHeader<N::BlockHeader>,
        attributes: OpNextBlockEnvAttributes,
        post_exec_mode: PostExecMode,
    ) -> OpBlockExecutionCtx {
        OpBlockExecutionCtx {
            parent_hash: parent.hash(),
            parent_beacon_block_root: attributes.parent_beacon_block_root,
            extra_data: attributes.extra_data,
            post_exec_mode,
        }
    }
}

impl<ChainSpec, N, R, EvmF> ConfigureEvm for FraxtalEvmConfig<ChainSpec, N, R, EvmF>
where
    ChainSpec: EthChainSpec<Header = Header> + OpHardforks,
    N: NodePrimitives<
            Receipt = R::Receipt,
            SignedTx = R::Transaction,
            BlockHeader = Header,
            BlockBody = alloy_consensus::BlockBody<R::Transaction>,
            Block = alloy_consensus::Block<R::Transaction>,
        >,
    OpTx: FromRecoveredTx<N::SignedTx> + FromTxWithEncoded<N::SignedTx>,
    N::SignedTx: OpConsensusTransaction,
    R: OpReceiptBuilder<
            Receipt: DepositReceipt,
            Transaction: SignedTransaction + OpConsensusTransaction,
        >,
    EvmF: EvmFactory<
            Tx: FromRecoveredTx<R::Transaction>
                    + FromTxWithEncoded<R::Transaction>
                    + alloy_evm::TransactionEnvMut
                    + OpTxEnv,
            Precompiles = PrecompilesMap,
            Spec = OpSpecId,
            BlockEnv = BlockEnv,
        > + Debug,
    FraxtalBlockExecutorFactory<R, Arc<ChainSpec>, EvmF>: for<'a> BlockExecutorFactory<
            EvmFactory = EvmF,
            ExecutionCtx<'a> = OpBlockExecutionCtx,
            Transaction = R::Transaction,
            Receipt = R::Receipt,
        >,
    Self: Send + Sync + Unpin + Clone + 'static,
{
    type Primitives = N;
    type Error = EIP1559ParamError;
    type NextBlockEnvCtx = OpNextBlockEnvAttributes;
    type BlockExecutorFactory = FraxtalBlockExecutorFactory<R, Arc<ChainSpec>, EvmF>;
    type BlockAssembler = OpBlockAssembler<ChainSpec>;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        &self.executor_factory
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        &self.block_assembler
    }

    fn evm_env(&self, header: &Header) -> Result<EvmEnv<OpSpecId>, Self::Error> {
        Ok(evm_env_for_op_block(header, self.chain_spec(), self.chain_spec().chain().id()))
    }

    fn next_evm_env(
        &self,
        parent: &Header,
        attributes: &Self::NextBlockEnvCtx,
    ) -> Result<EvmEnv<OpSpecId>, Self::Error> {
        Ok(evm_env_for_op_next_block(
            parent,
            NextEvmEnvAttributes {
                timestamp: attributes.timestamp,
                suggested_fee_recipient: attributes.suggested_fee_recipient,
                prev_randao: attributes.prev_randao,
                gas_limit: attributes.gas_limit,
                slot_number: None,
            },
            self.chain_spec().next_block_base_fee(parent, attributes.timestamp).unwrap_or_default(),
            self.chain_spec(),
            self.chain_spec().chain().id(),
        ))
    }

    fn context_for_block(
        &self,
        block: &'_ SealedBlock<N::Block>,
    ) -> Result<OpBlockExecutionCtx, Self::Error> {
        let post_exec_mode = post_exec_mode_from_transactions(
            block.body().transactions(),
            block.header().number(),
            self.is_sdm_active_at_timestamp(block.header().timestamp()),
        )?;

        Ok(self.context_for_block_with_post_exec_mode(block, Some(post_exec_mode)))
    }

    fn context_for_next_block(
        &self,
        parent: &SealedHeader<N::BlockHeader>,
        attributes: Self::NextBlockEnvCtx,
    ) -> Result<OpBlockExecutionCtx, Self::Error> {
        Ok(self.context_for_next_block_with_post_exec_mode(
            parent,
            attributes,
            PostExecMode::default(),
        ))
    }
}

impl<ChainSpec, N, R> ConfigureEngineEvm<OpExecutionData> for FraxtalEvmConfig<ChainSpec, N, R>
where
    ChainSpec: EthChainSpec<Header = Header> + OpHardforks,
    N: NodePrimitives<
            Receipt = R::Receipt,
            SignedTx = R::Transaction,
            BlockHeader = Header,
            BlockBody = alloy_consensus::BlockBody<R::Transaction>,
            Block = alloy_consensus::Block<R::Transaction>,
        >,
    OpTx: FromRecoveredTx<N::SignedTx> + FromTxWithEncoded<N::SignedTx>,
    N::SignedTx: Decodable2718 + OpConsensusTransaction,
    R: OpReceiptBuilder<
            Receipt: DepositReceipt,
            Transaction: SignedTransaction + OpConsensusTransaction,
        >,
    Self: Send + Sync + Unpin + Clone + 'static,
{
    fn evm_env_for_payload(
        &self,
        payload: &OpExecutionData,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        let timestamp = payload.payload.timestamp();
        let block_number = payload.payload.block_number();

        let spec = revm_spec_by_timestamp_after_bedrock(self.chain_spec(), timestamp);

        let cfg_env = CfgEnv::new()
            .with_chain_id(self.chain_spec().chain().id())
            .with_spec_and_mainnet_gas_params(spec);

        let blob_excess_gas_and_price = spec
            .into_eth_spec()
            .is_enabled_in(SpecId::CANCUN)
            .then_some(BlobExcessGasAndPrice { excess_blob_gas: 0, blob_gasprice: 1 });

        let block_env = BlockEnv {
            number: U256::from(block_number),
            beneficiary: payload.payload.as_v1().fee_recipient,
            timestamp: U256::from(timestamp),
            difficulty: if spec.into_eth_spec() >= SpecId::MERGE {
                U256::ZERO
            } else {
                payload.payload.as_v1().prev_randao.into()
            },
            prevrandao: (spec.into_eth_spec() >= SpecId::MERGE)
                .then(|| payload.payload.as_v1().prev_randao),
            gas_limit: payload.payload.as_v1().gas_limit,
            basefee: payload.payload.as_v1().base_fee_per_gas.to(),
            // EIP-4844 excess blob gas of this block, introduced in Cancun
            blob_excess_gas_and_price,
            slot_num: 0,
        };

        Ok(EvmEnv { cfg_env, block_env })
    }

    fn context_for_payload<'a>(
        &self,
        payload: &'a OpExecutionData,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        let transactions = payload
            .payload
            .transactions()
            .iter()
            .map(|encoded| TxTy::<Self::Primitives>::decode_2718_exact(encoded.as_ref()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| EIP1559ParamError::InvalidPostExecPayload)?;
        let post_exec_mode = post_exec_mode_from_transactions(
            transactions.iter(),
            payload.payload.block_number(),
            self.is_sdm_active_at_timestamp(payload.payload.timestamp()),
        )?;

        Ok(OpBlockExecutionCtx {
            parent_hash: payload.parent_hash(),
            parent_beacon_block_root: payload.sidecar.parent_beacon_block_root(),
            extra_data: payload.payload.as_v1().extra_data.clone(),
            post_exec_mode,
        })
    }

    fn tx_iterator_for_payload(
        &self,
        payload: &OpExecutionData,
    ) -> Result<impl ExecutableTxIterator<Self>, Self::Error> {
        let transactions = payload.payload.transactions().clone();
        let convert = |encoded: Bytes| {
            let tx = TxTy::<Self::Primitives>::decode_2718_exact(encoded.as_ref())
                .map_err(AnyError::new)?;
            let signer = tx.try_recover().map_err(AnyError::new)?;
            Ok::<_, AnyError>(WithEncoded::new(encoded, tx.with_signer(signer)))
        };

        Ok((transactions, convert))
    }
}

impl<ChainSpec, N, R> ConfigureEngineEvm<OpExecData> for FraxtalEvmConfig<ChainSpec, N, R>
where
    N: NodePrimitives,
    R: Send + Sync + Unpin + Clone + 'static,
    ChainSpec: Send + Sync + Unpin + Clone + 'static,
    Self: ConfigureEngineEvm<OpExecutionData>,
{
    fn evm_env_for_payload(&self, payload: &OpExecData) -> Result<EvmEnvFor<Self>, Self::Error> {
        ConfigureEngineEvm::<OpExecutionData>::evm_env_for_payload(self, &payload.0)
    }

    fn context_for_payload<'a>(
        &self,
        payload: &'a OpExecData,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        ConfigureEngineEvm::<OpExecutionData>::context_for_payload(self, &payload.0)
    }

    fn tx_iterator_for_payload(
        &self,
        payload: &OpExecData,
    ) -> Result<impl ExecutableTxIterator<Self>, Self::Error> {
        ConfigureEngineEvm::<OpExecutionData>::tx_iterator_for_payload(self, &payload.0)
    }
}
