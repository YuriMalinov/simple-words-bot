use std::env;

use anyhow::{Context, Result};
use teloxide::types::ChatId;

mod bot;
mod model;
mod utils;

#[tokio::main]
async fn main() -> Result<()> {
    if dotenv::from_filename(".env").is_err() {
        println!("No .env file found, working without it");
    }
    pretty_env_logger::init_timed();

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
    bot::setup_and_run_bot(bot::BotConfig {
        tasks,
        token,
        feedback_chat_id,
    })
    .await?;

    Ok(())
}
