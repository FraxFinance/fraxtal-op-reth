#![allow(missing_docs, rustdoc::missing_crate_level_docs)]
// The `optimism` feature must be enabled to use this crate.

use clap::Parser;
use fraxtal_chainspec::FraxtalChainSpecParser;
use fraxtal_node::node::FraxtalNode;
use reth_optimism_cli::Cli;
use reth_optimism_node::args::RollupArgs;
use tracing::info;

use tracing as _;

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

fn main() {
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    if let Err(err) =
        Cli::<FraxtalChainSpecParser, RollupArgs>::parse().run(|builder, rollup_args| async move {
            info!(target: "reth::cli", "Launching node");
            let handle = builder.launch_node(FraxtalNode::new(rollup_args)).await?;
            handle.node_exit_future.await
        })
    {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
