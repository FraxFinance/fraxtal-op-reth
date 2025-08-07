use std::collections::HashMap;

use alloy_op_hardforks::OpHardforks;
use alloy_primitives::{Address, B256, U256};
use reth_chainspec::EthChainSpec;
use revm::{
    database::State,
    state::{Account, Bytecode, EvmStorageSlot},
    Database, DatabaseCommit,
};
use tracing::info;

mod constants;

/// The Isthmus hardfork issues an irregular state transition that upgrades the remaining
/// frax tokens to upgreadable proxies.
pub(super) fn migrate_frax_isthmus<DB>(
    chain_spec: impl OpHardforks + EthChainSpec,
    timestamp: u64,
    db: &mut State<DB>,
) -> Result<(), DB::Error>
where
    DB: revm::Database,
{
    // If the granite hardfork is active at the current timestamp, and it was not active at the
    // previous block timestamp (heuristically, block time is not perfectly constant at 2s), and the
    // chain is an optimism chain, then we need to upgrade the oraacle contracts.
    if chain_spec.is_isthmus_active_at_timestamp(timestamp)
        && !chain_spec.is_isthmus_active_at_timestamp(timestamp.saturating_sub(2))
    {
        if chain_spec.chain_id() != 252 {
            return Ok(());
        }

        info!(target: "evm", "Forcing frax upgrades on Isthmus transition");

        for addr in constants::MAINNET_ORACLES_ADDRESSES {
            let mut implementation_addr = addr.clone();
            implementation_addr[0..3].copy_from_slice(&[252, 192, 211]);
            info!(target: "evm", "Setting implementation from {} to {}", addr, implementation_addr);

            let mut current_contract_acc = load_contract_account(db, *addr)?;
            let new_implementation_code = get_contract_code(db, &current_contract_acc);

            let mut implementation_acc = load_contract_account(db, implementation_addr)?;
            implementation_acc.code = Some(Bytecode::new_raw(new_implementation_code.into()));
            implementation_acc.code_hash = implementation_acc
                .code
                .as_ref()
                .unwrap_or(&Bytecode::new())
                .hash_slow();
            let mut implementation_revm_account: Account = implementation_acc.into();
            implementation_revm_account.mark_touch();

            let proxy_acc: revm::state::AccountInfo =
                load_contract_account(db, constants::PROXY_ADDR)?;
            current_contract_acc.code = proxy_acc.code.clone();
            current_contract_acc.code_hash = proxy_acc.code_hash.clone();

            let mut current_contract_revm_account: Account = current_contract_acc.into();
            current_contract_revm_account.mark_touch();
            info!(target: "evm", "Setting proxy {} admin to {}", constants::PROXY_ADDR, constants::PROXY_ADMIN_ADDR);
            current_contract_revm_account.storage.insert(
                U256::from_be_bytes(constants::PROXY_ADMIN_SLOT.into()),
                EvmStorageSlot::new_changed(
                    U256::default(),
                    U256::from_be_bytes(
                        B256::left_padding_from(constants::PROXY_ADMIN_ADDR.as_slice()).into(),
                    ),
                    0,
                ),
            );
            info!(target: "evm", "Setting proxy {} implementation to {}", constants::PROXY_ADDR, implementation_addr);
            current_contract_revm_account.storage.insert(
                U256::from_be_bytes(constants::PROXY_IMPLEMENTATION_SLOT.into()),
                EvmStorageSlot::new_changed(
                    U256::default(),
                    U256::from_be_bytes(
                        B256::left_padding_from(implementation_addr.as_slice()).into(),
                    ),
                    0,
                ),
            );

            db.commit(HashMap::from_iter([
                (implementation_addr, implementation_revm_account),
                (*addr, current_contract_revm_account),
            ]));
        }
    }
    Ok(())
}

fn load_contract_account<DB>(
    db: &mut State<DB>,
    address: Address,
) -> Result<revm::state::AccountInfo, DB::Error>
where
    DB: revm::Database,
{
    Ok(db
        .load_cache_account(address)?
        .account_info()
        .unwrap_or_default())
}

fn get_contract_code<DB>(db: &mut State<DB>, account: &revm::state::AccountInfo) -> Vec<u8>
where
    DB: revm::Database,
{
    account
        .code
        .clone()
        .unwrap_or_else(|| db.code_by_hash(account.code_hash).unwrap_or_default())
        .original_byte_slice()
        .to_owned()
}
