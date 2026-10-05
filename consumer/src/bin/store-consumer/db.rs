use clap::Args;
use postgres::{Client, Config, NoTls};
use std::error::Error;

#[derive(Args)]
pub struct DbArgs {
    #[arg(long, env = "POSTGRES_HOST")]
    postgres_host: String,
    #[arg(long, env = "POSTGRES_PORT", default_value_t = 5432)]
    postgres_port: u16,
    #[arg(long, env = "POSTGRES_DB")]
    postgres_db: String,
    #[arg(long, env = "POSTGRES_USER")]
    postgres_user: String,
    #[arg(long, env = "POSTGRES_PASSWORD")]
    postgres_password: String,
}

pub fn connect(args: &DbArgs) -> Result<Client, Box<dyn Error>> {
    let client = Config::new()
        .host(&args.postgres_host)
        .port(args.postgres_port)
        .dbname(&args.postgres_db)
        .user(&args.postgres_user)
        .password(&args.postgres_password)
        .connect(NoTls)?;
    Ok(client)
}
