use alloy_evm::Database;
use alloy_primitives::Address;

pub(super) fn load_contract_account<DB>(
    db: &mut DB,
    address: Address,
) -> Result<revm::state::AccountInfo, DB::Error>
where
    DB: Database,
{
    Ok(db.basic(address)?.unwrap_or_default())
}

pub(super) fn get_contract_code<DB>(db: &mut DB, account: &revm::state::AccountInfo) -> Vec<u8>
where
    DB: Database,
{
    account
        .code
        .clone()
        .unwrap_or_else(|| db.code_by_hash(account.code_hash).unwrap_or_default())
        .original_byte_slice()
        .to_owned()
}
