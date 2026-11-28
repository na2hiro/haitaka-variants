use clap::Parser;

fn main() -> anyhow::Result<()> {
    haitaka_cli::usi::run(haitaka_cli::usi::UsiArgs::parse())
}
