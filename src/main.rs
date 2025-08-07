use std::process::exit;

use clap::Parser;
mod handshake;
mod protocol;
mod zellij;

#[derive(Debug, Parser)]
pub struct JoinArgs {
    #[arg(help = "Peer to peer Node ID of the host you want to join")]
    host: String,
    #[arg(help = "Pre Shared Secret, also provided by the host")]
    secret: String,
}

#[derive(Parser, Debug)]
pub enum Command {
    Host,
    Join(JoinArgs),
}

#[tokio::main]
async fn main() {
    let args = Command::parse();
    let res = match args {
        Command::Host => handshake::handshake_host().await,
        Command::Join(args) => handshake::handshake_guest(&args.host, &args.secret).await,
    };
    if let Err(e) = res {
        println!("Error, terminated due to:");
        println!("{e}");
        exit(1);
    }
}
