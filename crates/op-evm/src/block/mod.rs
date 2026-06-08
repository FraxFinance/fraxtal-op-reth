//! Block executor for Fraxtal — wraps OP block executor and adds Fraxtal hardfork migrations.

use alloc::boxed::Box;
use alloy_consensus::{Transaction, TxReceipt};
use alloy_eips::Encodable2718;
use alloy_evm::{
    Database, Evm, EvmFactory, FromRecoveredTx, FromTxWithEncoded, IntoTxEnv,
    block::{
        BlockExecutionError, BlockExecutionResult, BlockExecutor, BlockExecutorFactory,
        ExecutableTx, GasOutput, OnStateHook, StateDB,
    },
};
use alloy_op_evm::{
    OpBlockExecutionCtx, OpBlockExecutor, OpBlockExecutorFactory, OpEvmFactory,
    block::{OpAlloyReceiptBuilder, OpTxEnv, receipt_builder::OpReceiptBuilder},
    post_exec::{
        PostExecEvm, PostExecEvmFactoryAdapter, PostExecEvmFactoryHooks, PostExecExecutorExt,
        WarmingRefundEvent, WarmingState,
    },
};
use alloy_op_hardforks::{OpChainHardforks, OpHardforks};
use op_alloy_consensus::{OpTransaction as OpConsensusTransaction, SDMGasEntry};
use op_revm::OpTransaction;
use reth_chainspec::EthChainSpec;
use revm::{
    DatabaseCommit, Inspector,
    context::{Block, TxEnv},
};

mod canyon;
mod granite;
mod holocene;
mod isthmus;
mod utils;

use canyon::ensure_create2_deployer;

/// Block executor for Fraxtal. Delegates to [`OpBlockExecutor`] and additionally applies the
/// Fraxtal-specific hardfork state migrations during pre-execution.
pub struct FraxtalBlockExecutor<E, R: OpReceiptBuilder, Spec> {
    inner: OpBlockExecutor<E, R, Spec>,
}

impl<E, R, Spec> core::fmt::Debug for FraxtalBlockExecutor<E, R, Spec>
where
    E: core::fmt::Debug,
    R: OpReceiptBuilder + core::fmt::Debug,
    R::Receipt: core::fmt::Debug,
    Spec: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FraxtalBlockExecutor").field("inner", &self.inner).finish()
    }
}

impl<E, R, Spec> FraxtalBlockExecutor<E, R, Spec>
where
    E: Evm,
    R: OpReceiptBuilder,
    Spec: OpHardforks + Clone,
{
    /// Creates a new [`FraxtalBlockExecutor`].
    pub fn new(evm: E, ctx: OpBlockExecutionCtx, spec: Spec, receipt_builder: R) -> Self {
        Self { inner: OpBlockExecutor::new(evm, ctx, spec, receipt_builder) }
    }
}

impl<E, R, Spec> BlockExecutor for FraxtalBlockExecutor<E, R, Spec>
where
    E: PostExecEvm<
            DB: Database + DatabaseCommit + StateDB,
            Tx: FromRecoveredTx<R::Transaction> + FromTxWithEncoded<R::Transaction> + OpTxEnv,
            HaltReason: Send + 'static,
        >,
    R: OpReceiptBuilder<
            Transaction: Transaction + Encodable2718 + OpConsensusTransaction,
            Receipt: TxReceipt,
        >,
    Spec: OpHardforks + EthChainSpec,
{
    type Transaction = R::Transaction;
    type Receipt = R::Receipt;
    type Evm = E;
    type Result = <OpBlockExecutor<E, R, Spec> as BlockExecutor>::Result;

    fn apply_pre_execution_changes(&mut self) -> Result<(), BlockExecutionError> {
        self.inner.apply_pre_execution_changes()?;

        let timestamp: u64 = self.inner.evm.block().timestamp().saturating_to();

        // Ensure that the create2deployer is force-deployed at the canyon transition. The OP block
        // executor already does this — re-running here is a no-op for OP chains, kept here for
        // robustness if a Fraxtal migration later relies on the create2 deployer being present.
        ensure_create2_deployer(&self.inner.spec, timestamp, self.inner.evm.db_mut())
            .map_err(BlockExecutionError::other)?;

        // Ensure that during the granite hard fork we migrate frax to frxUSD and sfrax to sfrxUSD
        granite::migrate_frxusd(&self.inner.spec, timestamp, self.inner.evm.db_mut())
            .map_err(BlockExecutionError::other)?;

        // Ensure that during the holocene hard fork we run the frax holocene migration
        holocene::migrate_frax_holocene(&self.inner.spec, timestamp, self.inner.evm.db_mut())
            .map_err(BlockExecutionError::other)?;

        // Ensure that during the isthmus hard fork we run the frax isthmus migration
        isthmus::migrate_frax_isthmus(&self.inner.spec, timestamp, self.inner.evm.db_mut())
            .map_err(BlockExecutionError::other)?;

        Ok(())
    }

    fn execute_transaction_without_commit(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<Self::Result, BlockExecutionError> {
        self.inner.execute_transaction_without_commit(tx)
    }

    fn commit_transaction(&mut self, output: Self::Result) -> GasOutput {
        self.inner.commit_transaction(output)
    }

    fn finish(
        self,
    ) -> Result<(Self::Evm, BlockExecutionResult<Self::Receipt>), BlockExecutionError> {
        self.inner.finish()
    }

    fn set_state_hook(&mut self, hook: Option<Box<dyn OnStateHook>>) {
        self.inner.set_state_hook(hook);
    }

    fn evm_mut(&mut self) -> &mut Self::Evm {
        self.inner.evm_mut()
    }

    fn evm(&self) -> &Self::Evm {
        self.inner.evm()
    }

    fn receipts(&self) -> &[Self::Receipt] {
        self.inner.receipts()
    }
}

impl<E, R, Spec> PostExecExecutorExt for FraxtalBlockExecutor<E, R, Spec>
where
    E: PostExecEvm,
    R: OpReceiptBuilder,
    Spec: OpHardforks + Clone,
{
    fn post_exec_entries(&self) -> &[SDMGasEntry] {
        self.inner.post_exec_entries()
    }

    fn take_post_exec_entries(&mut self) -> alloc::vec::Vec<SDMGasEntry> {
        self.inner.take_post_exec_entries()
    }

    fn take_warming_events_by_tx(
        &mut self,
    ) -> alloc::vec::Vec<alloc::vec::Vec<WarmingRefundEvent>> {
        self.inner.take_warming_events_by_tx()
    }

    fn warming_state(&self) -> WarmingState {
        self.inner.warming_state()
    }

    fn seed_warming_state(&mut self, state: WarmingState) {
        self.inner.seed_warming_state(state)
    }
}

/// Fraxtal block executor factory. Wraps [`OpBlockExecutorFactory`] and produces
/// [`FraxtalBlockExecutor`]s.
#[derive(Debug, Clone, Default, Copy)]
pub struct FraxtalBlockExecutorFactory<
    R = OpAlloyReceiptBuilder,
    Spec = OpChainHardforks,
    EvmFactory = OpEvmFactory,
> {
    inner: OpBlockExecutorFactory<R, Spec, EvmFactory>,
}

impl<R, Spec, EvmFactory> FraxtalBlockExecutorFactory<R, Spec, EvmFactory> {
    /// Creates a new [`FraxtalBlockExecutorFactory`].
    pub const fn new(receipt_builder: R, spec: Spec, evm_factory: EvmFactory) -> Self {
        Self { inner: OpBlockExecutorFactory::new(receipt_builder, spec, evm_factory) }
    }

    /// Exposes the receipt builder.
    pub const fn receipt_builder(&self) -> &R {
        self.inner.receipt_builder()
    }

    /// Exposes the chain specification.
    pub const fn spec(&self) -> &Spec {
        self.inner.spec()
    }

    /// Exposes the EVM factory.
    pub const fn evm_factory(&self) -> &EvmFactory {
        self.inner.evm_factory()
    }
}

impl<R, Spec, F> BlockExecutorFactory
    for FraxtalBlockExecutorFactory<R, Spec, PostExecEvmFactoryAdapter<F>>
where
    R: OpReceiptBuilder<
            Transaction: Transaction + Encodable2718 + OpConsensusTransaction,
            Receipt: TxReceipt,
        > + 'static,
    Spec: OpHardforks + EthChainSpec + 'static,
    F: PostExecEvmFactoryHooks + 'static,
    F::Tx: FromRecoveredTx<R::Transaction> + FromTxWithEncoded<R::Transaction> + OpTxEnv,
    Self: 'static,
{
    type EvmFactory = PostExecEvmFactoryAdapter<F>;
    type ExecutionCtx<'a> = OpBlockExecutionCtx;
    type Transaction = R::Transaction;
    type Receipt = R::Receipt;
    type TxExecutionResult = <OpBlockExecutorFactory<R, Spec, PostExecEvmFactoryAdapter<F>> as BlockExecutorFactory>::TxExecutionResult;
    type Executor<
        'a,
        DB: StateDB,
        I: Inspector<<PostExecEvmFactoryAdapter<F> as EvmFactory>::Context<DB>>,
    > = FraxtalBlockExecutor<
        <PostExecEvmFactoryAdapter<F> as EvmFactory>::Evm<DB, I>,
        &'a R,
        &'a Spec,
    >;

    fn evm_factory(&self) -> &Self::EvmFactory {
        self.inner.evm_factory()
    }

    fn create_executor<'a, DB, I>(
        &'a self,
        evm: <PostExecEvmFactoryAdapter<F> as EvmFactory>::Evm<DB, I>,
        ctx: Self::ExecutionCtx<'a>,
    ) -> Self::Executor<'a, DB, I>
    where
        DB: StateDB,
        I: Inspector<<PostExecEvmFactoryAdapter<F> as EvmFactory>::Context<DB>>,
    {
        FraxtalBlockExecutor::new(evm, ctx, self.inner.spec(), self.inner.receipt_builder())
    }
}

impl<R, Spec, Tx> BlockExecutorFactory for FraxtalBlockExecutorFactory<R, Spec, OpEvmFactory<Tx>>
where
    R: OpReceiptBuilder<
            Transaction: Transaction + Encodable2718 + OpConsensusTransaction,
            Receipt: TxReceipt,
        > + 'static,
    Spec: OpHardforks + EthChainSpec + 'static,
    Tx: IntoTxEnv<Tx>
        + Into<OpTransaction<TxEnv>>
        + Default
        + Clone
        + core::fmt::Debug
        + FromRecoveredTx<R::Transaction>
        + FromTxWithEncoded<R::Transaction>
        + OpTxEnv
        + 'static,
    Self: 'static,
{
    type EvmFactory = OpEvmFactory<Tx>;
    type ExecutionCtx<'a> = OpBlockExecutionCtx;
    type Transaction = R::Transaction;
    type Receipt = R::Receipt;
    type TxExecutionResult = <OpBlockExecutorFactory<R, Spec, OpEvmFactory<Tx>> as BlockExecutorFactory>::TxExecutionResult;
    type Executor<'a, DB: StateDB, I: Inspector<<OpEvmFactory<Tx> as EvmFactory>::Context<DB>>> =
        FraxtalBlockExecutor<<OpEvmFactory<Tx> as EvmFactory>::Evm<DB, I>, &'a R, &'a Spec>;

    fn evm_factory(&self) -> &Self::EvmFactory {
        self.inner.evm_factory()
    }

    fn create_executor<'a, DB, I>(
        &'a self,
        evm: <OpEvmFactory<Tx> as EvmFactory>::Evm<DB, I>,
        ctx: Self::ExecutionCtx<'a>,
    ) -> Self::Executor<'a, DB, I>
    where
        DB: StateDB,
        I: Inspector<<OpEvmFactory<Tx> as EvmFactory>::Context<DB>>,
    {
        FraxtalBlockExecutor::new(evm, ctx, self.inner.spec(), self.inner.receipt_builder())
    }
}
