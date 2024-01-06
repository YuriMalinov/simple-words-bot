use indoc::indoc;
use rand::seq::SliceRandom;
use teloxide::{
    payloads::SendMessageSetters,
    requests::{Request, Requester},
    types::{ChatId, ParseMode},
    Bot,
};

use crate::{model::Task, utils::escape_telegram_symbols};

use super::{
    ask_next_task_handler::ask_next_task,
    bot_core::{BotContext, UserData},
    bot_filter::{collect_filter_info, match_task, parse_filter},
};

#[derive(Debug, thiserror::Error)]
pub(super) enum FilterErrors {}

pub(super) async fn handle_filter(
    bot: &Bot,
    command_text: Option<&str>,
    chat_id: ChatId,
    context: &BotContext,
) -> anyhow::Result<()> {
    match command_text {
        Some(text) => change_filter(bot, text, chat_id, context).await,
        None => handle_filter_help(bot, chat_id, context).await,
    }
}

async fn change_filter(
    bot: &Bot,
    filter_text: &str,
    chat_id: ChatId,
    context: &BotContext,
) -> anyhow::Result<()> {
    if filter_text == "-" {
        {
            let mut state = context.user_data.lock().unwrap();
            let user_state = state.entry(chat_id).or_default();
            user_state.filter = None;
            user_state.current_tasks = vec![];
        }
        ask_next_task(bot, context, chat_id).await?;
        return Ok(());
    }

    let filter = parse_filter(filter_text);
    let task_ids = context
        .tasks
        .iter()
        .filter(|task| match_task(&task.filters, &filter))
        .map(|task| task.id)
        .collect::<Vec<_>>();

    if task_ids.is_empty() {
        bot.send_message(
            chat_id,
            "Ничего не найдено по фильтру, попробуйте изменить его",
        )
        .await?;
    } else {
        {
            let mut state = context.user_data.lock().unwrap();
            let user_state = state.entry(chat_id).or_default();
            user_state.filter = Some(filter_text.into());
            user_state.current_tasks = vec![];
        }

        ask_next_task(bot, context, chat_id).await?;
    }
    Ok(())
}

pub(super) fn select_tasks(user_data: &mut UserData, tasks: &[Task]) -> anyhow::Result<Vec<u64>> {
    let mut current_tasks = if let Some(filter) = &user_data.filter {
        let filter = parse_filter(filter);
        tasks
            .iter()
            .filter(|task| match_task(&task.filters, &filter))
            .map(|task| task.id)
            .collect::<Vec<_>>()
    } else {
        tasks.iter().map(|task| task.id).collect::<Vec<_>>()
    };
    current_tasks.shuffle(&mut rand::thread_rng());
    Ok(current_tasks)
}

async fn handle_filter_help(
    bot: &Bot,
    chat_id: ChatId,
    context: &BotContext,
) -> anyhow::Result<()> {
    let mut message = indoc! {r#"
        Фильтр позволяет выбрать задания по определенным критериям.
        Например, можно выбрать все задания, c падежом genetiv.

        Перечислите через запятую значения, которые нужно оставить. Например, `/filter genetiv, accusative`.

        Чтобы задание удовлетворяло нескольким критериям, перечислите их через точку с запятой. Например, `/filter genetiv, accusative; plural`.

        Чтобы сбросить фильтр, используйте `/filter-reset` или `/filter -`.
        
        Возможные значения:
    "#}.to_owned();

    let filter_info = collect_filter_info(&context.tasks);
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
