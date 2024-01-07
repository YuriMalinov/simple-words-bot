use std::time;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use prost::Message;
use rand::seq::{IteratorRandom, SliceRandom};
use rand::thread_rng;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, ParseMode, ReplyMarkup};
use teloxide::Bot;

use crate::bot::bot_core::BotErrors;
use crate::bot::bot_filter::parse_filter;
use crate::model::Task;
use crate::utils::{escape_telegram_symbols, rus_numeric};

use super::bot_core::{BotContext, TaskInfoService, UserStateService};
use super::proto;

pub async fn ask_next_task<T: TaskInfoService, U: UserStateService>(
    bot: &Bot,
    context: &BotContext<T, U>,
    chat_id: ChatId,
) -> anyhow::Result<()> {
    let notify;
    let current_filter;
    let task = {
        let mut user_data = context.user_data.get_state(chat_id).await?;
        current_filter = user_data.filter.clone();

        notify = if user_data.current_tasks.is_empty() {
            let filter = current_filter.as_ref().map(|f| parse_filter(f));
            user_data.current_tasks = context.tasks.get_task_ids(filter.as_ref()).await?;
            user_data.current_tasks.shuffle(&mut thread_rng());
            Some(user_data.current_tasks.len())
        } else {
            None
        };

        let task_id = user_data.current_tasks.pop().ok_or(BotErrors::NoTaskFound)?;

        context.user_data.update_state(chat_id, user_data).await?;

        context
            .tasks
            .get_task(task_id)
            .await
            .transpose()
            .ok_or(BotErrors::NoTaskFound)?
    }?;

    if let Some(generated_tasks) = notify {
        bot.send_message(
            chat_id,
            format!(
                "У меня есть {generated_tasks} {tasks}{filter}, поехали\\!\n\n_Напоминаю, задачи сгенерированы автоматически и могут содержать ошибки\\. Хотя мы очень старались, чтобы это происходило пореже\\._",
                tasks = rus_numeric(generated_tasks, "задач", "задача", "задачи"),
                filter = match current_filter {
                    Some(filter) =>
                        format!(" по фильтру `{filter}` (используйте /filter, чтобы поменять)"),
                    None => "".to_owned(),
                }
            ),
        )
        .parse_mode(ParseMode::MarkdownV2)
        .send()
        .await?;
    }

    let MessageData { message, buttons } = build_message(&task)?;
    log::debug!(
        "#{chat_id} asking: {}",
        message[QUESTION_PRELUDE.len()..].trim().lines().next().unwrap_or_default()
    );

    let message_data = message.clone();
    let result = bot
        .send_message(chat_id, message)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(ReplyMarkup::inline_kb(
            buttons
                .into_iter()
                .map(|button| vec![InlineKeyboardButton::callback(button.text, button.command)])
                .collect::<Vec<_>>(),
        ))
        .send()
        .await;

    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            log::error!("Error sending message {message_data}: {e}");
            Err(e.into())
        }
    }
}

#[derive(Debug, PartialEq)]
struct SimpleCommand {
    text: String,
    command: String,
}

#[derive(Debug, PartialEq)]
struct MessageData {
    message: String,
    buttons: Vec<SimpleCommand>,
}

pub const QUESTION_PRELUDE: &str = "➖❔➖❔➖❔➖❔➖❔➖\n\n\n";

fn build_message(task: &Task) -> anyhow::Result<MessageData> {
    let mut message = QUESTION_PRELUDE.to_owned();
    message.push_str(&replace_mask_with_base_word(&task.masked_task, &task.base));
    message.push('\n');

    for info in &task.info {
        message.push_str("\n\n_");
        message.push_str(info);
        message.push_str("_\n");
    }

    for hint in &task.hints {
        message.push('\n');
        message.push_str(&hint.name);
        message.push_str(": ||");
        message.push_str(&hint.value);
        message.push_str("||");
    }

    message = escape_telegram_symbols(&message, ".-!()");

    let mut variants = vec![(&task.correct, true)];
    variants.extend(
        task.wrong_answers
            .iter()
            .filter(|v| **v != task.correct)
            .choose_multiple(&mut thread_rng(), 3)
            .into_iter()
            .map(|answer| (answer, false)),
    );
    variants.shuffle(&mut thread_rng());

    let buttons = variants
        .iter()
        .enumerate()
        .map(|(i, (variant, correct))| {
            let command = proto::Command {
                command: Some(proto::command::Command::QuestionAnswer(proto::QuestionAnswer {
                    task_id: task.id,
                    index: i as i32,
                    is_correct: *correct,
                    time_asked_ts: time::SystemTime::now().duration_since(time::UNIX_EPOCH).unwrap().as_millis() as i64,
                })),
            };
            SimpleCommand {
                text: (*variant).clone(),
                command: STANDARD.encode(command.encode_to_vec()),
            }
        })
        .collect::<Vec<_>>();

    Ok(MessageData { message, buttons })
}

fn replace_mask_with_base_word(sentence: &str, base: &str) -> String {
    let mut result = String::new();
    let words: Vec<&str> = base.split(' ').filter(|w| !w.is_empty()).collect();
    let parts: Vec<&str> = sentence.split("*****").collect();

    for (i, part) in parts.iter().enumerate() {
        result.push_str(part);
        if i == parts.len() - 1 {
            break;
        }

        result.push_str("`[");
        if i >= words.len() {
            result.push_str("?????");
            log::warn!("Not enough words in base: {base} for sentence {sentence}");
        } else if i + 2 < parts.len() {
            result.push_str(words[i]);
        } else {
            result.push_str(&words[i..words.len()].join(" "));
        }
        result.push_str("]`");
    }

    result
}

#[cfg(test)]
mod test {
    #[test]
    fn test_replace_mask_with_base_word() {
        let sentence = "Ovo je *****.";
        let base = "moja  kuća";
        let result = super::replace_mask_with_base_word(sentence, base);
        assert_eq!(result, "Ovo je `moja kuća`.");
    }

    #[test]
    fn test_replace_several_masks() {
        let sentence = "Ovo je ***** *****.";
        let base = "moja  kuća";
        let result = super::replace_mask_with_base_word(sentence, base);
        assert_eq!(result, "Ovo je `moja` `kuća`.");
    }

    #[test]
    fn test_replace_not_enough_words() {
        let sentence = "Ovo je ***** *****.";
        let base = "moja";
        let result = super::replace_mask_with_base_word(sentence, base);
        assert_eq!(result, "Ovo je `moja` `?????`.");
    }
}
