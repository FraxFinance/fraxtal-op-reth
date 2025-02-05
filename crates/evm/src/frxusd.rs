use std::{collections::HashMap, sync::Arc};

use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_forks::OpHardfork;
use revm::{db::State, DatabaseCommit};
use revm_primitives::{address, b256, Account, Address, Bytecode, EvmStorageSlot, B256, U256};
use tracing::info;

const FRAX_ADDR: Address = address!("Fc00000000000000000000000000000000000001");
const FRAX_IMPL_ADDR: Address = address!("fcc0d30000000000000000000000000000000001");
const SFRAX_ADDR: Address = address!("Fc00000000000000000000000000000000000008");
const SFRAX_IMPL_ADDR: Address = address!("fcc0d30000000000000000000000000000000008");
const PROXY_ADDR: Address = address!("fc0000000000000000000000000000000000000a");
const PROXY_ADMIN_ADDR: Address = address!("fc0000000000000000000000000000000000000a");
const FRXUSD_L1_ADDR: Address = address!("CAcd6fd266aF91b8AeD52aCCc382b4e165586E29");
const SFRXUSD_L1_ADDR: Address = address!("cf62F905562626CfcDD2261162a51fd02Fc9c5b6");

const PROXY_ADMIN_SLOT: B256 =
    b256!("b53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103");
const PROXY_IMPLEMENTATION_SLOT: B256 =
    b256!("360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc");
const FRXUSD_NAME_STORAGE_VALUE: B256 =
    b256!("4672617820555344000000000000000000000000000000000000000000000010");
const FRXUSD_SYMBOL_STORAGE_VALUE: B256 =
    b256!("667278555344000000000000000000000000000000000000000000000000000c");
const SFRXUSD_NAME_STORAGE_VALUE: B256 =
    b256!("5374616b6564204672617820555344000000000000000000000000000000001e");
const SFRXUSD_SYMBOL_STORAGE_VALUE: B256 =
    b256!("736672785553440000000000000000000000000000000000000000000000000e");

const MAINNET_FRAX_L1_REPLACEMENTS_INDEXES: &[usize] = &[2137, 6850, 7218];
const MAINNET_SFRAX_L1_REPLACEMENTS_INDEXES: &[usize] = &[2137, 6850, 7218];

const TESTNET_FRAX_L1_REPLACEMENTS_INDEXES: &[usize] = &[2137, 6962, 7330];

const DEVNET_FRAX_L1_REPLACEMENTS_INDEXES: &[usize] = &[693, 1303];
const DEVNET_SFRAX_L1_REPLACEMENTS_INDEXES: &[usize] = &[693, 1303];

/// The Graanite hardfork issues an irregular state transition that upgrades the frax/sfrax
/// contracts code to be upgreadable proxies.
pub fn ensure_frxusd<DB>(
    chain_spec: Arc<OpChainSpec>,
    timestamp: u64,
    db: &mut revm::State<DB>,
) -> Result<(), DB::Error>
where
    DB: revm::Database,
{
    // If the granite hardfork is active at the current timestamp, and it was not active at the
    // previous block timestamp (heuristically, block time is not perfectly constant at 2s), and the
    // chain is an optimism chain, then we need to upgrade the frax/sfrax contracts.
    if chain_spec.is_fork_active_at_timestamp(OpHardfork::Granite, timestamp)
        && !chain_spec.is_fork_active_at_timestamp(OpHardfork::Granite, timestamp.saturating_sub(2))
    {
        info!(target: "evm", "Forcing frxusd upgrade on Granite transition");

        match chain_spec.chain().id() {
            2521 => {
                migrate(
                    FRAX_ADDR,
                    FRAX_IMPL_ADDR,
                    PROXY_ADDR,
                    FRXUSD_L1_ADDR,
                    DEVNET_FRAX_L1_REPLACEMENTS_INDEXES,
                    PROXY_ADMIN_ADDR,
                    FRXUSD_NAME_STORAGE_VALUE,
                    FRXUSD_SYMBOL_STORAGE_VALUE,
                    db,
                )?;
                migrate(
                    SFRAX_ADDR,
                    SFRAX_IMPL_ADDR,
                    PROXY_ADDR,
                    SFRXUSD_L1_ADDR,
                    DEVNET_SFRAX_L1_REPLACEMENTS_INDEXES,
                    PROXY_ADMIN_ADDR,
                    SFRXUSD_NAME_STORAGE_VALUE,
                    SFRXUSD_SYMBOL_STORAGE_VALUE,
                    db,
                )?;
            }
            2522 => {
                migrate(
                    FRAX_ADDR,
                    FRAX_IMPL_ADDR,
                    PROXY_ADDR,
                    FRXUSD_L1_ADDR,
                    TESTNET_FRAX_L1_REPLACEMENTS_INDEXES,
                    PROXY_ADMIN_ADDR,
                    FRXUSD_NAME_STORAGE_VALUE,
                    FRXUSD_SYMBOL_STORAGE_VALUE,
                    db,
                )?;
            }
            _ => {
                migrate(
                    FRAX_ADDR,
                    FRAX_IMPL_ADDR,
                    PROXY_ADDR,
                    FRXUSD_L1_ADDR,
                    MAINNET_FRAX_L1_REPLACEMENTS_INDEXES,
                    PROXY_ADMIN_ADDR,
                    FRXUSD_NAME_STORAGE_VALUE,
                    FRXUSD_SYMBOL_STORAGE_VALUE,
                    db,
                )?;
                migrate(
                    SFRAX_ADDR,
                    SFRAX_IMPL_ADDR,
                    PROXY_ADDR,
                    SFRXUSD_L1_ADDR,
                    MAINNET_SFRAX_L1_REPLACEMENTS_INDEXES,
                    PROXY_ADMIN_ADDR,
                    SFRXUSD_NAME_STORAGE_VALUE,
                    SFRXUSD_SYMBOL_STORAGE_VALUE,
                    db,
                )?;
            }
        }

        return Ok(());
    }

    Ok(())
}

fn migrate<DB>(
    contract_addr: Address,
    implementation_addr: Address,
    proxy_source_addr: Address,
    l1_token: Address,
    l1_bytecode_replacmenets: &[usize],
    owner: Address,
    name_storage: B256,
    symbol_storage: B256,
    db: &mut State<DB>,
) -> Result<(), DB::Error>
where
    DB: revm::Database,
{
    info!(target: "evm", "Setting implementation from {} to {}", contract_addr, implementation_addr);
    let l1_token_bytes = l1_token.as_slice();
    let mut current_contract_acc = db
        .load_cache_account(contract_addr)?
        .account_info()
        .unwrap_or_default();

    let mut new_implementation_code = current_contract_acc
        .code
        .unwrap_or_default()
        .bytes_slice()
        .to_owned();

    for i in l1_bytecode_replacmenets {
        new_implementation_code[*i..].copy_from_slice(l1_token_bytes);
    }

    let mut implementation_acc = db
        .load_cache_account(implementation_addr)?
        .account_info()
        .unwrap_or_default();
    implementation_acc.code = Some(Bytecode::new_raw(new_implementation_code.into()));
    implementation_acc.code_hash = implementation_acc
        .code
        .as_ref()
        .unwrap_or(&Bytecode::new())
        .hash_slow();
    let mut implementation_revm_account: Account = implementation_acc.into();
    implementation_revm_account.mark_touch();

    info!(target: "evm", "Setting proxy from {} to {}", proxy_source_addr, contract_addr);
    let proxy_acc = db
        .load_cache_account(proxy_source_addr)?
        .account_info()
        .unwrap_or_default();
    current_contract_acc.code = proxy_acc.code.clone();
    current_contract_acc.code_hash = proxy_acc.code_hash.clone();

    let mut current_contract_revm_account: Account = current_contract_acc.into();
    current_contract_revm_account.mark_touch();
    info!(target: "evm", "Setting proxy {} admin to {}", proxy_source_addr, owner);
    current_contract_revm_account.storage.insert(
        U256::from_be_bytes(PROXY_ADMIN_SLOT.into()),
        EvmStorageSlot::new_changed(
            U256::default(),
            U256::from_be_bytes(B256::left_padding_from(owner.as_slice()).into()),
        ),
    );
    info!(target: "evm", "Setting proxy {} implementation to {}", proxy_source_addr, implementation_addr);
    current_contract_revm_account.storage.insert(
        U256::from_be_bytes(PROXY_IMPLEMENTATION_SLOT.into()),
        EvmStorageSlot::new_changed(
            U256::default(),
            U256::from_be_bytes(B256::left_padding_from(implementation_addr.as_slice()).into()),
        ),
    );

    info!(target: "evm", "Setting proxy {} name and symbol", proxy_source_addr);
    current_contract_revm_account.storage.insert(
        U256::from(3),
        EvmStorageSlot::new_changed(U256::default(), U256::from_be_bytes(name_storage.into())),
    );
    current_contract_revm_account.storage.insert(
        U256::from(4),
        EvmStorageSlot::new_changed(U256::default(), U256::from_be_bytes(symbol_storage.into())),
    );

    db.commit(HashMap::from_iter([
        (implementation_addr, implementation_revm_account),
        (contract_addr, current_contract_revm_account),
    ]));

    Ok(())
}
