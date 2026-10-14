#![allow(missing_docs, rustdoc::missing_crate_level_docs)]
// The `optimism` feature must be enabled to use this crate.

use fraxtal_chainspec::FraxtalChainSpecParser;
use fraxtal_node::proof_history::launch_node;
use reth_optimism_cli::Cli;
use reth_optimism_node::args::RollupArgs;
use tracing::info;

use tracing as _;

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

fn main() {
    reth_cli_util::sigsegv_handler::install();

    if std::env::var_os("RUST_BACKTRACE").is_none() {
        unsafe {
            std::env::set_var("RUST_BACKTRACE", "1");
        }
    }

    if let Err(err) = Cli::<FraxtalChainSpecParser, RollupArgs>::parse_with_denied_args().run(
        async move |builder, rollup_args| {
            info!(target: "reth::cli", "Launching node");
            launch_node(builder, rollup_args).await
        },
    ) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
