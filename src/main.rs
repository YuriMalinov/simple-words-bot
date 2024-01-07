use std::env;

use anyhow::{Context, Result};
use bot::{
    bot_services_in_mem::{LocalTasks, LocalUserStateService},
    BotConfig,
};
use service::user_state::PgUserService;
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

    let connection_url = env::var("DATABASE_URL").context("No DATABASE_URL environment")?;
    let pool = sqlx::postgres::PgPool::connect(&connection_url).await?;
    sqlx::migrate!().run(&pool).await?;

    let data_dir = env::var("DATA_DIR").unwrap_or("data".to_owned());

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
        bot::setup_and_run_bot(
            BotConfig {
                token,
                feedback_chat_id,
            },
            LocalTasks::new(tasks),
            PgUserService::new(pool),
        )
        .await?;
    }

    Ok(())
}
