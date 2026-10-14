extern crate alloc;

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
    parse_post_exec_payload_from_transactions, validate_post_exec_entry_count,
};
use op_revm::OpSpecId;
use reth_chainspec::EthChainSpec;
use reth_evm::{ConfigureEvm, EvmEnv, eth::NextEvmEnvAttributes, precompiles::PrecompilesMap};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::{
    NullRefundPolicy, OpBlockAssembler, OpNextBlockEnvAttributes, OpRethReceiptBuilder,
    PostExecMode,
};
use reth_optimism_forks::OpHardforks;
use reth_optimism_primitives::{DepositReceipt, OpPrimitives};
use reth_primitives_traits::{NodePrimitives, SealedBlock, SealedHeader, SignedTransaction};
use revm::context::BlockEnv;

#[allow(unused_imports)]
use {
    alloy_eips::Decodable2718,
    alloy_primitives::Bytes,
    op_alloy_rpc_types_engine::OpExecutionData,
    reth_evm::{EvmEnvFor, ExecutionCtxFor},
    reth_optimism_payload_builder::OpExecData,
    reth_primitives_traits::{TxTy, WithEncoded},
    reth_storage_errors::any::AnyError,
};

#[cfg(feature = "std")]
use {
    alloy_op_evm::evm_env_for_op_payload,
    reth_evm::{ConfigureEngineEvm, ExecutableTxIterator},
};

mod post_exec_ext;

mod sdm_metrics;

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

impl<ChainSpec, N: NodePrimitives, R, EvmFactory> FraxtalEvmConfig<ChainSpec, N, R, EvmFactory> {
    /// Creates a new [`FraxtalEvmConfig`] with an explicit EVM factory.
    pub fn new_with_evm_factory(
        chain_spec: Arc<ChainSpec>,
        receipt_builder: R,
        evm_factory: EvmFactory,
    ) -> Self {
        Self {
            block_assembler: OpBlockAssembler::new(chain_spec.clone()),
            executor_factory: FraxtalBlockExecutorFactory::new(
                receipt_builder,
                chain_spec,
                evm_factory,
            ),
            _pd: core::marker::PhantomData,
        }
    }
}

impl<ChainSpec: EthChainSpec<Header = Header> + OpHardforks, N: NodePrimitives, R>
    FraxtalEvmConfig<ChainSpec, N, R>
{
    /// Creates a new [`FraxtalEvmConfig`] with the given chain spec.
    pub fn new(chain_spec: Arc<ChainSpec>, receipt_builder: R) -> Self {
        Self::new_with_evm_factory(
            chain_spec,
            receipt_builder,
            FraxtalEvmFactory::<OpTx, NullRefundPolicy>::default(),
        )
    }
}

/// Returns true when SDM post-exec transactions are consensus-active at `timestamp`.
///
/// Defers to the hardfork where SDM is activated, matching op-node's `IsSDM` and kona's
/// `is_sdm_active`.
///
/// Single source of truth for the SDM protocol gate: call sites holding only a chain spec should
/// route through this rather than calling the underlying fork accessor directly.
pub fn is_sdm_active_at_timestamp(chain_spec: &impl OpHardforks, timestamp: u64) -> bool {
    chain_spec.is_lagoon_active_at_timestamp(timestamp)
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
        .inspect_err(|error| sdm_metrics::report_post_exec_validation_failure(block_number, *error))
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
            // No parent header on this path to detect fork-activation blocks, so the executor's
            // check is skipped; the derivation layer enforces the rule instead.
            no_user_tx_activation_block: false,
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
            no_user_tx_activation_block: self
                .chain_spec()
                .is_no_user_tx_activation_block(parent.timestamp(), attributes.timestamp),
            parent_beacon_block_root: attributes.parent_beacon_block_root,
            extra_data: attributes.extra_data,
            post_exec_mode,
        }
    }
}

/// UPSTREAM-MIRROR(copy): reth@rev:0fbe428 `reth_evm_ethereum::EthEvmConfig`
///
/// Mirrors upstream `ConfigureEvm` plumbing with OP environments and execution context.
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

/// UPSTREAM-MIRROR(copy): reth@rev:0fbe428 `reth_evm_ethereum::EthEvmConfig`
///
/// Mirrors upstream payload-to-EVM configuration with OP payload and fork semantics.
/// `tx_iterator_for_payload` recovers senders directly; upstream routes the same recovery through
/// an optional `SenderRecoveryCache`, which only memoizes `try_recover` and so returns the same
/// signer.
#[cfg(feature = "std")]
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
        Ok(evm_env_for_op_payload(
            &payload.payload,
            self.chain_spec(),
            self.chain_spec().chain().id(),
        ))
    }

    fn context_for_payload<'a>(
        &self,
        payload: &'a OpExecutionData,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        validate_post_exec_entry_count(payload.payload.transactions())
            .map_err(|_| EIP1559ParamError::InvalidPostExecPayload)?;
        let transactions = payload
            .payload
            .transactions()
            .iter()
            .map(|encoded| TxTy::<Self::Primitives>::decode_2718_exact(encoded.as_ref()))
            .collect::<Result<Vec<_>, _>>()
            .inspect_err(|error| {
                tracing::warn!(
                    block_number = payload.payload.block_number(),
                    %error,
                    "payload rejected: transaction failed to decode"
                );
            })
            .map_err(|_| EIP1559ParamError::InvalidPostExecPayload)?;
        let post_exec_mode = post_exec_mode_from_transactions(
            transactions.iter(),
            payload.payload.block_number(),
            self.is_sdm_active_at_timestamp(payload.payload.timestamp()),
        )?;

        Ok(OpBlockExecutionCtx {
            parent_hash: payload.payload.parent_hash(),
            // No parent header on this path to detect fork-activation blocks, so the executor's
            // check is skipped; the derivation layer enforces the rule instead.
            no_user_tx_activation_block: false,
            parent_beacon_block_root: payload.sidecar.parent_beacon_block_root(),
            extra_data: payload.payload.as_v1().extra_data.clone(),
            post_exec_mode,
        })
    }

    fn tx_iterator_for_payload(
        &self,
        payload: &OpExecutionData,
    ) -> Result<impl ExecutableTxIterator<Self>, Self::Error> {
        validate_post_exec_entry_count(payload.payload.transactions())
            .map_err(|_| EIP1559ParamError::InvalidPostExecPayload)?;
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

#[cfg(feature = "std")]
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
