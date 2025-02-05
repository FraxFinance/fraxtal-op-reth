//! Optimism block execution strategy.

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use alloy_consensus::{Eip658Value, Header, Receipt, Transaction as _};
use alloy_eips::eip7685::Requests;
use core::fmt::Display;
use op_alloy_consensus::{OpDepositReceipt, OpTxType};
use reth_chainspec::EthereumHardforks;
use reth_consensus::ConsensusError;
use reth_evm::{
    env::EvmEnv,
    execute::{
        balance_increment_state, BasicBlockExecutorProvider, BlockExecutionError,
        BlockExecutionStrategy, BlockExecutionStrategyFactory, BlockValidationError, ExecuteOutput,
        ProviderError,
    },
    state_change::post_block_balance_increments,
    system_calls::{OnStateHook, SystemCaller},
    ConfigureEvm, TxEnvOverrides,
};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_consensus::validate_block_post_execution;
use reth_optimism_evm::{l1::ensure_create2_deployer, OpBlockExecutionError, OpEvmConfig};
use reth_optimism_forks::OpHardfork;
use reth_optimism_primitives::{OpBlock, OpPrimitives, OpReceipt, OpTransactionSigned};
use reth_primitives::BlockWithSenders;
use reth_primitives_traits::SignedTransaction;
use reth_revm::{Database, State};
use revm_primitives::{db::DatabaseCommit, EnvWithHandlerCfg, ResultAndState};
use tracing::trace;

use crate::{error::FraxtalBlockExecutionError, frxusd::ensure_frxusd};

/// Factory for [`FraxtalExecutionStrategy`].
#[derive(Debug, Clone)]
pub struct FraxtalExecutionStrategyFactory<EvmConfig = OpEvmConfig> {
    /// The chainspec
    chain_spec: Arc<OpChainSpec>,
    /// How to create an EVM.
    evm_config: EvmConfig,
}

impl FraxtalExecutionStrategyFactory {
    /// Creates a new default optimism executor strategy factory.
    pub fn optimism(chain_spec: Arc<OpChainSpec>) -> Self {
        Self::new(chain_spec.clone(), OpEvmConfig::new(chain_spec))
    }
}

impl<EvmConfig> FraxtalExecutionStrategyFactory<EvmConfig> {
    /// Creates a new executor strategy factory.
    pub const fn new(chain_spec: Arc<OpChainSpec>, evm_config: EvmConfig) -> Self {
        Self {
            chain_spec,
            evm_config,
        }
    }
}

impl<EvmConfig> BlockExecutionStrategyFactory for FraxtalExecutionStrategyFactory<EvmConfig>
where
    EvmConfig: Clone
        + Unpin
        + Sync
        + Send
        + 'static
        + ConfigureEvm<Header = alloy_consensus::Header, Transaction = OpTransactionSigned>,
{
    type Primitives = OpPrimitives;
    type Strategy<DB: Database<Error: Into<ProviderError> + Display>> =
        FraxtalExecutionStrategy<DB, EvmConfig>;

    fn create_strategy<DB>(&self, db: DB) -> Self::Strategy<DB>
    where
        DB: Database<Error: Into<ProviderError> + Display>,
    {
        let state = State::builder()
            .with_database(db)
            .with_bundle_update()
            .without_state_clear()
            .build();
        FraxtalExecutionStrategy::new(state, self.chain_spec.clone(), self.evm_config.clone())
    }
}

/// Block execution strategy for Optimism.
#[allow(missing_debug_implementations)]
pub struct FraxtalExecutionStrategy<DB, EvmConfig>
where
    EvmConfig: Clone,
{
    /// The chainspec
    chain_spec: Arc<OpChainSpec>,
    /// How to create an EVM.
    evm_config: EvmConfig,
    /// Optional overrides for the transactions environment.
    tx_env_overrides: Option<Box<dyn TxEnvOverrides>>,
    /// Current state for block execution.
    state: State<DB>,
    /// Utility to call system smart contracts.
    system_caller: SystemCaller<EvmConfig, OpChainSpec>,
}

impl<DB, EvmConfig> FraxtalExecutionStrategy<DB, EvmConfig>
where
    EvmConfig: Clone,
{
    /// Creates a new [`FraxtalExecutionStrategy`]
    pub fn new(state: State<DB>, chain_spec: Arc<OpChainSpec>, evm_config: EvmConfig) -> Self {
        let system_caller = SystemCaller::new(evm_config.clone(), chain_spec.clone());
        Self {
            state,
            chain_spec,
            evm_config,
            system_caller,
            tx_env_overrides: None,
        }
    }
}

impl<DB, EvmConfig> FraxtalExecutionStrategy<DB, EvmConfig>
where
    DB: Database<Error: Into<ProviderError> + Display>,
    EvmConfig: ConfigureEvm<Header = alloy_consensus::Header>,
{
    /// Configures a new evm configuration and block environment for the given block.
    ///
    /// Caution: this does not initialize the tx environment.
    fn evm_env_for_block(&self, header: &Header) -> EnvWithHandlerCfg {
        let evm_env = self.evm_config.cfg_and_block_env(header);
        let EvmEnv {
            cfg_env_with_handler_cfg,
            block_env,
        } = evm_env;
        EnvWithHandlerCfg::new_with_cfg_env(cfg_env_with_handler_cfg, block_env, Default::default())
    }
}

impl<DB, EvmConfig> BlockExecutionStrategy for FraxtalExecutionStrategy<DB, EvmConfig>
where
    DB: Database<Error: Into<ProviderError> + Display>,
    EvmConfig: ConfigureEvm<Header = alloy_consensus::Header, Transaction = OpTransactionSigned>,
{
    type DB = DB;
    type Primitives = OpPrimitives;
    type Error = BlockExecutionError;

    fn init(&mut self, tx_env_overrides: Box<dyn TxEnvOverrides>) {
        self.tx_env_overrides = Some(tx_env_overrides);
    }

    fn apply_pre_execution_changes(
        &mut self,
        block: &BlockWithSenders<OpBlock>,
    ) -> Result<(), Self::Error> {
        // Set state clear flag if the block is after the Spurious Dragon hardfork.
        let state_clear_flag =
            (*self.chain_spec).is_spurious_dragon_active_at_block(block.header.number);
        self.state.set_state_clear_flag(state_clear_flag);

        let env = self.evm_env_for_block(&block.header);
        let mut evm = self.evm_config.evm_with_env(&mut self.state, env);

        self.system_caller.apply_beacon_root_contract_call(
            block.timestamp,
            block.number,
            block.parent_beacon_block_root,
            &mut evm,
        )?;

        // Ensure that the create2deployer is force-deployed at the canyon transition. Optimism
        // blocks will always have at least a single transaction in them (the L1 info transaction),
        // so we can safely assume that this will always be triggered upon the transition and that
        // the above check for empty blocks will never be hit on OP chains.
        ensure_create2_deployer(self.chain_spec.clone(), block.timestamp, evm.db_mut())
            .map_err(|_| OpBlockExecutionError::ForceCreate2DeployerFail)?;

        // Ensure that frxUSD is force-deployed at the granite transition
        ensure_frxusd(self.chain_spec.clone(), block.timestamp, evm.db_mut())
            .map_err(|_| FraxtalBlockExecutionError::ForceFrxUSDFail)?;
        Ok(())
    }

    fn execute_transactions(
        &mut self,
        block: &BlockWithSenders<OpBlock>,
    ) -> Result<ExecuteOutput<OpReceipt>, Self::Error> {
        let env = self.evm_env_for_block(&block.header);
        let mut evm = self.evm_config.evm_with_env(&mut self.state, env);

        let is_regolith = self
            .chain_spec
            .fork(OpHardfork::Regolith)
            .active_at_timestamp(block.timestamp);

        let mut cumulative_gas_used = 0;
        let mut receipts = Vec::with_capacity(block.body.transactions.len());
        for (sender, transaction) in block.transactions_with_sender() {
            // The sum of the transaction’s gas limit, Tg, and the gas utilized in this block prior,
            // must be no greater than the block’s gasLimit.
            let block_available_gas = block.header.gas_limit - cumulative_gas_used;
            if transaction.gas_limit() > block_available_gas
                && (is_regolith || !transaction.is_deposit())
            {
                return Err(
                    BlockValidationError::TransactionGasLimitMoreThanAvailableBlockGas {
                        transaction_gas_limit: transaction.gas_limit(),
                        block_available_gas,
                    }
                    .into(),
                );
            }

            // Cache the depositor account prior to the state transition for the deposit nonce.
            //
            // Note that this *only* needs to be done post-regolith hardfork, as deposit nonces
            // were not introduced in Bedrock. In addition, regular transactions don't have deposit
            // nonces, so we don't need to touch the DB for those.
            let depositor = (is_regolith && transaction.is_deposit())
                .then(|| {
                    evm.db_mut()
                        .load_cache_account(*sender)
                        .map(|acc| acc.account_info().unwrap_or_default())
                })
                .transpose()
                .map_err(|_| OpBlockExecutionError::AccountLoadFailed(*sender))?;

            self.evm_config
                .fill_tx_env(evm.tx_mut(), transaction, *sender);

            if let Some(tx_env_overrides) = &mut self.tx_env_overrides {
                tx_env_overrides.apply(evm.tx_mut());
            }

            // Execute transaction.
            let result_and_state = evm.transact().map_err(move |err| {
                let new_err = err.map_db_err(|e| e.into());
                // Ensure hash is calculated for error log, if not already done
                BlockValidationError::EVM {
                    hash: transaction.recalculate_hash(),
                    error: Box::new(new_err),
                }
            })?;

            trace!(
                target: "evm",
                ?transaction,
                "Executed transaction"
            );
            self.system_caller.on_state(&result_and_state.state);
            let ResultAndState { result, state } = result_and_state;
            evm.db_mut().commit(state);

            // append gas used
            cumulative_gas_used += result.gas_used();

            let receipt = Receipt {
                // Success flag was added in `EIP-658: Embedding transaction status code in
                // receipts`.
                status: Eip658Value::Eip658(result.is_success()),
                cumulative_gas_used,
                logs: result.into_logs(),
            };

            // Push transaction changeset and calculate header bloom filter for receipt.
            receipts.push(match transaction.tx_type() {
                OpTxType::Legacy => OpReceipt::Legacy(receipt),
                OpTxType::Eip2930 => OpReceipt::Eip2930(receipt),
                OpTxType::Eip1559 => OpReceipt::Eip1559(receipt),
                OpTxType::Eip7702 => OpReceipt::Eip7702(receipt),
                OpTxType::Deposit => OpReceipt::Deposit(OpDepositReceipt {
                    inner: receipt,
                    deposit_nonce: depositor.map(|account| account.nonce),
                    // The deposit receipt version was introduced in Canyon to indicate an update to
                    // how receipt hashes should be computed when set. The state
                    // transition process ensures this is only set for
                    // post-Canyon deposit transactions.
                    deposit_receipt_version: (transaction.is_deposit()
                        && self
                            .chain_spec
                            .is_fork_active_at_timestamp(OpHardfork::Canyon, block.timestamp))
                    .then_some(1),
                }),
            });
        }

        Ok(ExecuteOutput {
            receipts,
            gas_used: cumulative_gas_used,
        })
    }

    fn apply_post_execution_changes(
        &mut self,
        block: &BlockWithSenders<OpBlock>,
        _receipts: &[OpReceipt],
    ) -> Result<Requests, Self::Error> {
        let balance_increments =
            post_block_balance_increments(&self.chain_spec.clone(), &block.block);
        // increment balances
        self.state
            .increment_balances(balance_increments.clone())
            .map_err(|_| BlockValidationError::IncrementBalanceFailed)?;
        // call state hook with changes due to balance increments.
        let balance_state = balance_increment_state(&balance_increments, &mut self.state)?;
        self.system_caller.on_state(&balance_state);

        Ok(Requests::default())
    }

    fn state_ref(&self) -> &State<DB> {
        &self.state
    }

    fn state_mut(&mut self) -> &mut State<DB> {
        &mut self.state
    }

    fn with_state_hook(&mut self, hook: Option<Box<dyn OnStateHook>>) {
        self.system_caller.with_state_hook(hook);
    }

    fn validate_block_post_execution(
        &self,
        block: &BlockWithSenders<OpBlock>,
        receipts: &[OpReceipt],
        _requests: &Requests,
    ) -> Result<(), ConsensusError> {
        validate_block_post_execution(block, &self.chain_spec.clone(), receipts)
    }
}

/// Helper type with backwards compatible methods to obtain executor providers.
#[derive(Debug)]
pub struct OpExecutorProvider;

impl OpExecutorProvider {
    /// Creates a new default optimism executor strategy factory.
    pub fn optimism(
        chain_spec: Arc<OpChainSpec>,
    ) -> BasicBlockExecutorProvider<FraxtalExecutionStrategyFactory> {
        BasicBlockExecutorProvider::new(FraxtalExecutionStrategyFactory::optimism(chain_spec))
    }
}
