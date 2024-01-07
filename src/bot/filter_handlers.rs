use indoc::indoc;
use teloxide::{
    payloads::SendMessageSetters,
    requests::{Request, Requester},
    types::{ChatId, ParseMode},
    Bot,
};

use crate::utils::escape_telegram_symbols;

use super::{
    ask_next_task_handler::ask_next_task,
    bot_core::{BotContext, TaskInfoService, UserStateService},
    bot_filter::parse_filter,
};

#[derive(Debug, thiserror::Error)]
pub(super) enum FilterErrors {}

pub(super) async fn handle_filter<T: TaskInfoService, U: UserStateService>(
    bot: &Bot,
    command_text: Option<&str>,
    chat_id: ChatId,
    context: &BotContext<T, U>,
) -> anyhow::Result<()> {
    match command_text {
        Some(text) => change_filter(bot, text, chat_id, context).await,
        None => handle_filter_help(bot, chat_id, context).await,
    }
}

async fn change_filter<T: TaskInfoService, U: UserStateService>(
    bot: &Bot,
    filter_text: &str,
    chat_id: ChatId,
    context: &BotContext<T, U>,
) -> anyhow::Result<()> {
    if filter_text == "-" {
        {
            let mut user_state = context.user_data.get_state(chat_id).await?;
            user_state.filter = None;
            user_state.current_tasks = vec![];
            context.user_data.update_state(chat_id, user_state).await?;
        }
        ask_next_task(bot, context, chat_id).await?;
        return Ok(());
    }

    let filter = parse_filter(filter_text);
    let task_ids = context.tasks.get_task_ids(Some(&filter)).await?;

    if task_ids.is_empty() {
        bot.send_message(chat_id, "Ничего не найдено по фильтру, попробуйте изменить его")
            .await?;
    } else {
        {
            let mut user_state = context.user_data.get_state(chat_id).await?;
            user_state.filter = Some(filter_text.into());
            user_state.current_tasks = vec![];
            context.user_data.update_state(chat_id, user_state).await?;
        }

        ask_next_task(bot, context, chat_id).await?;
    }
    Ok(())
}

async fn handle_filter_help<T: TaskInfoService, U: UserStateService>(
    bot: &Bot,
    chat_id: ChatId,
    context: &BotContext<T, U>,
) -> anyhow::Result<()> {
    let mut message = indoc! {r#"
        Фильтр позволяет выбрать задания по определенным критериям.
        Например, можно выбрать все задания, c падежом genetiv.

        Перечислите через запятую значения, которые нужно оставить. Например, `/filter genetiv, accusative`.

        Чтобы задание удовлетворяло нескольким критериям, перечислите их через точку с запятой. Например, `/filter genetiv, accusative; plural`.

        Чтобы сбросить фильтр, используйте `/filter-reset` или `/filter -`.
        
        Возможные значения:
    "#}.to_owned();

    let filter_info = context.tasks.collect_filter_info().await?;
    for filter in filter_info {
        let values = filter.possible_values.join(", ");
        let composed = format!("- {}: {}\n", filter.name, values);
        message.push_str(&composed);
    }

    let message = escape_telegram_symbols(&message, ".-*_()[]");
    bot.send_message(chat_id, message)
        .parse_mode(ParseMode::MarkdownV2)
        .send()
        .await?;
    Ok(())
}
