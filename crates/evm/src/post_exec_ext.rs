use alloc::{sync::Arc, vec::Vec};
use alloy_consensus::Header;
use alloy_evm::{FromRecoveredTx, FromTxWithEncoded, block::BlockExecutor};
use alloy_op_evm::{
    block::{OpTxEnv, receipt_builder::OpReceiptBuilder},
    post_exec::{PostExecEvmFactoryAdapter, PostExecEvmFactoryHooks, PostExecExecutorExt},
};
use core::fmt::Debug;
use fraxtal_op_evm::{FraxtalBlockExecutor, FraxtalBlockExecutorFactory, FraxtalEvmFactory};
use op_alloy_consensus::OpTransaction as OpConsensusTransaction;
use op_revm::OpSpecId;
use reth_chainspec::EthChainSpec;
use reth_evm::{
    ConfigureEvm as _, Database,
    execute::{BasicBlockBuilder, BlockBuilder},
    precompiles::PrecompilesMap,
};
use reth_optimism_evm::{ConfigurePostExecEvm, OpTx, PostExecMode};
use reth_optimism_forks::OpHardforks;
use reth_optimism_primitives::DepositReceipt;
use reth_primitives_traits::{NodePrimitives, SealedBlock, SealedHeader, SignedTransaction};
use revm::{context::BlockEnv, database::State};

use crate::FraxtalEvmConfig;

impl<ChainSpec, N, R> ConfigurePostExecEvm for FraxtalEvmConfig<ChainSpec, N, R>
where
    ChainSpec: EthChainSpec<Header = Header> + OpHardforks + Send + Sync + Unpin + 'static,
    N: NodePrimitives<
            Receipt = R::Receipt,
            SignedTx = R::Transaction,
            BlockHeader = Header,
            BlockBody = alloy_consensus::BlockBody<R::Transaction>,
            Block = alloy_consensus::Block<R::Transaction>,
        >,
    OpTx: FromRecoveredTx<N::SignedTx> + FromTxWithEncoded<N::SignedTx>,
    R: OpReceiptBuilder<
            Receipt: DepositReceipt,
            Transaction: SignedTransaction + OpConsensusTransaction,
        > + Clone
        + Send
        + Sync
        + Unpin
        + 'static,
    Self: Send + Sync + Unpin + Clone + 'static,
{
    fn post_exec_executor_for_block<'a, DB: Database>(
        &'a self,
        db: &'a mut State<DB>,
        block: &'a SealedBlock<<Self::Primitives as NodePrimitives>::Block>,
        post_exec_mode: PostExecMode,
    ) -> Result<
        impl BlockExecutor<
            Transaction = <Self::Primitives as NodePrimitives>::SignedTx,
            Receipt = <Self::Primitives as NodePrimitives>::Receipt,
        > + PostExecExecutorExt
        + 'a,
        Self::Error,
    > {
        let evm = self.evm_for_block(db, block.header())?;
        let ctx = self.context_for_block_with_post_exec_mode(block, Some(post_exec_mode));

        Ok(FraxtalBlockExecutor::new(
            evm,
            ctx,
            self.executor_factory.spec(),
            self.executor_factory.receipt_builder(),
        ))
    }

    fn post_exec_builder_for_next_block<'a, DB: Database + 'a>(
        &'a self,
        db: &'a mut State<DB>,
        parent: &'a SealedHeader<<Self::Primitives as NodePrimitives>::BlockHeader>,
        attributes: Self::NextBlockEnvCtx,
        post_exec_mode: PostExecMode,
    ) -> Result<
        impl BlockBuilder<
            Primitives = Self::Primitives,
            Executor: PostExecExecutorExt
                          + BlockExecutor<
                Evm: alloy_evm::Evm<DB: core::ops::DerefMut<Target = State<DB>>>,
            >,
        > + 'a,
        Self::Error,
    > {
        let evm_env = self.next_evm_env(parent, &attributes)?;
        let evm = self.evm_with_env(db, evm_env);
        let ctx =
            self.context_for_next_block_with_post_exec_mode(parent, attributes, post_exec_mode);
        let executor = FraxtalBlockExecutor::new(
            evm,
            ctx.clone(),
            self.executor_factory.spec(),
            self.executor_factory.receipt_builder(),
        );

        Ok(BasicBlockBuilder::<
            'a,
            FraxtalBlockExecutorFactory<R, Arc<ChainSpec>, FraxtalEvmFactory<OpTx>>,
            _,
            _,
            N,
        > {
            executor,
            transactions: Vec::new(),
            ctx,
            parent,
            assembler: self.block_assembler(),
        })
    }
}

impl<ChainSpec, N, R, F> ConfigurePostExecEvm
    for FraxtalEvmConfig<ChainSpec, N, R, PostExecEvmFactoryAdapter<F>>
where
    ChainSpec: EthChainSpec<Header = Header> + OpHardforks + Send + Sync + Unpin + 'static,
    N: NodePrimitives<
            Receipt = R::Receipt,
            SignedTx = R::Transaction,
            BlockHeader = Header,
            BlockBody = alloy_consensus::BlockBody<R::Transaction>,
            Block = alloy_consensus::Block<R::Transaction>,
        >,
    OpTx: FromRecoveredTx<N::SignedTx> + FromTxWithEncoded<N::SignedTx>,
    R: OpReceiptBuilder<
            Receipt: DepositReceipt,
            Transaction: SignedTransaction + OpConsensusTransaction,
        > + Clone
        + Send
        + Sync
        + Unpin
        + 'static,
    F: PostExecEvmFactoryHooks<
            Tx: FromRecoveredTx<R::Transaction>
                    + FromTxWithEncoded<R::Transaction>
                    + alloy_evm::TransactionEnvMut
                    + OpTxEnv,
            Precompiles = PrecompilesMap,
            Spec = OpSpecId,
            BlockEnv = BlockEnv,
        > + Debug
        + Clone
        + Send
        + Sync
        + Unpin
        + 'static,
    Self: Send + Sync + Unpin + Clone + 'static,
{
    fn post_exec_executor_for_block<'a, DB: Database>(
        &'a self,
        db: &'a mut State<DB>,
        block: &'a SealedBlock<<Self::Primitives as NodePrimitives>::Block>,
        post_exec_mode: PostExecMode,
    ) -> Result<
        impl BlockExecutor<
            Transaction = <Self::Primitives as NodePrimitives>::SignedTx,
            Receipt = <Self::Primitives as NodePrimitives>::Receipt,
        > + PostExecExecutorExt
        + 'a,
        Self::Error,
    > {
        let evm = self.evm_for_block(db, block.header())?;
        let ctx = self.context_for_block_with_post_exec_mode(block, Some(post_exec_mode));

        Ok(FraxtalBlockExecutor::new(
            evm,
            ctx,
            self.executor_factory.spec(),
            self.executor_factory.receipt_builder(),
        ))
    }

    fn post_exec_builder_for_next_block<'a, DB: Database + 'a>(
        &'a self,
        db: &'a mut State<DB>,
        parent: &'a SealedHeader<<Self::Primitives as NodePrimitives>::BlockHeader>,
        attributes: Self::NextBlockEnvCtx,
        post_exec_mode: PostExecMode,
    ) -> Result<
        impl BlockBuilder<
            Primitives = Self::Primitives,
            Executor: PostExecExecutorExt
                          + BlockExecutor<
                Evm: alloy_evm::Evm<DB: core::ops::DerefMut<Target = State<DB>>>,
            >,
        > + 'a,
        Self::Error,
    > {
        let evm_env = self.next_evm_env(parent, &attributes)?;
        let evm = self.evm_with_env(db, evm_env);
        let ctx =
            self.context_for_next_block_with_post_exec_mode(parent, attributes, post_exec_mode);
        let executor = FraxtalBlockExecutor::new(
            evm,
            ctx.clone(),
            self.executor_factory.spec(),
            self.executor_factory.receipt_builder(),
        );

        Ok(BasicBlockBuilder::<
            'a,
            FraxtalBlockExecutorFactory<R, Arc<ChainSpec>, PostExecEvmFactoryAdapter<F>>,
            _,
            _,
            N,
        > {
            executor,
            transactions: Vec::new(),
            ctx,
            parent,
            assembler: self.block_assembler(),
        })
    }
}
