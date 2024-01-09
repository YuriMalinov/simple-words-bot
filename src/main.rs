use std::{env, path::Path};

use anyhow::{Context, Result};
use bot::{
    bot_services_in_mem::{LocalTasks, LocalUserStateService},
    BotConfig,
};
use service::task_info_service::PgTaskInfoService;
use service::user_state::PgUserService;
use sqlx::postgres::{PgConnectOptions, PgPool};
use teloxide::types::ChatId;

mod bot;
mod model;
mod service;
#[cfg(test)]
mod test_db;
mod utils;

#[tokio::main]
async fn main() -> Result<()> {
    if dotenv::from_filename(".env").is_err() {
        println!("No .env file found, working without it");
    }
    pretty_env_logger::init_timed();

    log::info!("Connecting to database...");
    let connection_url = env::var("DATABASE_URL").context("No DATABASE_URL environment")?;

    let pgcert = Path::new(".pgcert");
    let connect_options = if pgcert.exists() {
        log::info!("Found .pgcert file, using it for SSL connection");
        let pgcert = std::fs::read(pgcert)?;
        connection_url
            .parse::<PgConnectOptions>()?
            .ssl_root_cert_from_pem(pgcert)
            .ssl_mode(sqlx::postgres::PgSslMode::VerifyCa)
    } else {
        connection_url.parse::<PgConnectOptions>()?
    };

    let pool = PgPool::connect_with(connect_options).await?;
    sqlx::migrate!().run(&pool).await?;

    let data_dir = env::var("DATA_DIR").unwrap_or("data".to_owned());

    log::info!("Reading tasks from {data_dir}...");
    let task_groups = model::scan_data_directory(&data_dir)?;
    let tasks = task_groups
        .into_iter()
        .flat_map(|task_group| task_group.tasks.into_iter())
        .collect::<Vec<_>>();

    let token = env::var("TELEGRAM_BOT_TOKEN").context("No TELEGRAM_BOT_TOKEN environment")?;
    let feedback_chat_id = env::var("FEEDBACK_CHAT_ID")
        .context("No FEEDBACK_CHAT_ID environment")?
        .parse::<i64>()
        .map(ChatId)
        .ok();

    log::info!("Got {} tasks, starting bot.", tasks.len());
    let args: Vec<String> = env::args().collect();
    if args.get(1) == Some(&"local".to_owned()) {
        bot::setup_and_run_bot(
            BotConfig {
                token,
                feedback_chat_id,
            },
            LocalTasks::new(tasks),
            LocalUserStateService::default(),
        )
        .await?;
    } else {
        let task_info_service = PgTaskInfoService::new(pool.clone());
        task_info_service.update_tasks(&tasks).await.context("Failed to update tasks")?;

        bot::setup_and_run_bot(
            BotConfig {
                token,
                feedback_chat_id,
            },
            task_info_service,
            PgUserService::new(pool.clone()),
        )
        .await?;
    }

    Ok(())
}
