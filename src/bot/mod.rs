pub use crate::bot::bot_core::{setup_and_run_bot, BotConfig};

mod ask_next_task_handler;
mod bot_core;
mod bot_filter;
mod filter_handlers;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/bot.proto.rs"));
}
