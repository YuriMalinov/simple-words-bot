use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use indoc::indoc;
use rand::seq::{IteratorRandom, SliceRandom};
use rand::thread_rng;
use teloxide::dptree::deps;
use teloxide::prelude::*;
use teloxide::types::{
    InlineKeyboardButton, InlineKeyboardButtonKind, InlineKeyboardMarkup, MessageEntity,
    MessageEntityKind, ParseMode, ReplyMarkup,
};
use teloxide::Bot;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::model::Task;
use crate::utils::rus_numeric;

#[derive(Debug)]
struct BotContext {
    tasks: Vec<Task>,
    user_data: Mutex<HashMap<ChatId, UserData>>,
}

#[derive(Debug)]
struct UserData {
    current_tasks: Vec<u64>,
}

#[derive(Error, Debug)]
enum BotErrors {
    #[error("No task found")]
    NoTaskFound,
    #[error("No message found")]
    NoMessageFound,
    #[error("No data")]
    NoData,
    #[error("Wrong query")]
    WrongQuery,
    #[error("Bad callback data in button")]
    BadCallbackDataInButton,
    #[error("No reply markup")]
    NoReplyMarkup,
}

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub async fn setup_and_run_bot(tasks: Vec<Task>, token: &str) -> Result<()> {
    let bot = Bot::new(token);

    let user_data = Mutex::new(HashMap::new());
    let context = BotContext { tasks, user_data };

    Dispatcher::builder(
        bot,
        dptree::entry()
            .branch(Update::filter_message().endpoint(handle_message))
            .branch(Update::filter_callback_query().endpoint(handle_callback_query)),
    )
    .dependencies(deps![Arc::new(context)])
    .enable_ctrlc_handler()
    .build()
    .dispatch()
    .await;

    Ok(())
}

static HELP_TEXT: &str = indoc! {"
    Hi there! This bot will help you to learn cases in Serbian language (or at least try to).            

    You can start by typing /start command. Return to this message with /help or any other text.
    "};

async fn handle_message(bot: Bot, message: Message, context: Arc<BotContext>) -> HandlerResult {
    let chat_id = message.chat.id;
    handle(&bot.clone(), chat_id, || async move {
        let text = message
            .text()
            .ok_or(anyhow::anyhow!("Not a text message"))?;

        match text.trim() {
            "/start" => {
                ask_next_task(&bot, context.clone(), chat_id).await?;
            }
            _ => {
                bot.send_message(chat_id, HELP_TEXT).send().await?;
            }
        }
        Ok(())
    })
    .await
}

async fn ask_next_task(bot: &Bot, context: Arc<BotContext>, chat_id: ChatId) -> Result<()> {
    // context
    //     .tasks
    //     .choose(&mut thread_rng())
    //     .ok_or(BotErrors::NoTaskFound)?;
    let notify;
    let task = {
        let mut user_data = context.user_data.lock().await;
        let user_data = user_data.entry(chat_id).or_insert_with(|| UserData {
            current_tasks: vec![],
        });

        notify = if user_data.current_tasks.is_empty() {
            user_data.current_tasks = context.tasks.iter().map(|task| task.id).collect::<Vec<_>>();
            user_data.current_tasks.shuffle(&mut thread_rng());
            Some(user_data.current_tasks.len())
        } else {
            None
        };

        let task_id = user_data
            .current_tasks
            .pop()
            .ok_or(BotErrors::NoTaskFound)?;

        context
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or(BotErrors::NoTaskFound)?
    };

    if let Some(generated_tasks) = notify {
        bot.send_message(
            chat_id,
            format!(
                "У меня есть {generated_tasks} {tasks}, поехали!",
                tasks = rus_numeric(generated_tasks, "задач", "задача", "задачи")
            ),
        )
        .send()
        .await?;
    }

    let MessageData { message, buttons } = build_message(task)?;
    log::debug!(
        "#{chat_id} asking: {}",
        message[QUESTION_PRELUDE.len()..]
            .trim()
            .lines()
            .next()
            .unwrap_or_default()
    );

    bot.send_message(chat_id, message)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(ReplyMarkup::inline_kb(
            buttons
                .into_iter()
                .map(|button| vec![InlineKeyboardButton::callback(button.text, button.command)])
                .collect::<Vec<_>>(),
        ))
        .send()
        .await?;

    Ok(())
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

const QUESTION_PRELUDE: &str = "➖❔➖❔➖❔➖❔➖❔➖\n\n\n";

fn build_message(task: &Task) -> Result<MessageData> {
    let mut message = QUESTION_PRELUDE.to_owned();
    message.push_str(&replace_mask_with_base_word(
        &task.masked_sentence,
        &task.base,
    ));

    message.push_str("\n\n_");
    message.push_str(&task.sentence_ru);
    message.push_str("_\n");

    for hint in &task.hints {
        message.push('\n');
        message.push_str(&hint.name);
        message.push_str(": ||");
        message.push_str(&hint.value);
        message.push_str("||");
    }

    message = message.replace('.', "\\.").replace('-', "\\-");

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
        .map(|(i, (variant, correct))| SimpleCommand {
            text: (*variant).clone(),
            command: format!("{i}:{correct}"),
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

async fn handle_callback_query(
    bot: Bot,
    query: CallbackQuery,
    context: Arc<BotContext>,
) -> HandlerResult {
    let message = query.message.as_ref().ok_or(BotErrors::NoMessageFound)?;
    let chat_id = message.chat.id;
    handle(&bot.clone(), chat_id, || async move {
        bot.answer_callback_query(query.id).send().await?;

        let data = query.data.ok_or(BotErrors::NoData)?;
        let parts = data.split(':').collect::<Vec<_>>();
        if parts.len() != 2 {
            return Err(BotErrors::WrongQuery.into());
        }

        let (answer_index, correct) = parse_command(data.as_str())?;
        log::debug!("#{chat_id} got answer correct={correct}");

        let buttons = &message
            .reply_markup()
            .ok_or(BotErrors::NoReplyMarkup)
            .context("reply_markup")?
            .inline_keyboard;

        let mut correct_text = &buttons[0][0].text;
        let mut answer_text = &buttons[0][0].text;
        for button in buttons {
            let (button_index, correct) =
                if let InlineKeyboardButtonKind::CallbackData(command) = &button[0].kind {
                    parse_command(command)?
                } else {
                    return Err(BotErrors::BadCallbackDataInButton.into());
                };

            if button_index == answer_index {
                answer_text = &button[0].text;
            }
            if correct {
                correct_text = &button[0].text;
            }
        }

        let text = message.text().ok_or(BotErrors::NoMessageFound)?;
        let (mut text, entities_offset) = if let Some(prefix) = text.strip_prefix(QUESTION_PRELUDE)
        {
            (prefix.to_owned(), QUESTION_PRELUDE.chars().count())
        } else {
            (text.to_owned(), 0)
        };

        text.push_str("\n\n");
        if !correct {
            text.push_str("\n❌ ");
            text.push_str(answer_text);
        }
        text.push_str("\n✅ ");
        text.push_str(correct_text);

        bot.edit_message_reply_markup(chat_id, message.id)
            .reply_markup(InlineKeyboardMarkup::default())
            .send()
            .await?;

        let fixed_entities = message.entities().map(|entities| {
            entities
                .iter()
                .filter(|entity| !matches!(entity.kind, MessageEntityKind::Spoiler))
                .map(|entity| MessageEntity {
                    kind: entity.kind.clone(),
                    offset: entity.offset - entities_offset,
                    length: entity.length,
                })
                .collect::<Vec<_>>()
        });

        let mut call = bot.edit_message_text(chat_id, message.id, text);
        if let Some(entities) = fixed_entities {
            call = call.entities(entities)
        }
        call.send().await?;

        tokio::time::sleep(Duration::from_secs(1)).await;

        ask_next_task(&bot, context, chat_id).await?;

        Ok(())
    })
    .await
}

fn parse_command(command: &str) -> Result<(usize, bool)> {
    let parts = command.split(':').collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err(BotErrors::WrongQuery.into());
    }
    let index = parts[0]
        .parse::<usize>()
        .map_err(|_| BotErrors::WrongQuery)?;
    let correct = parts[1]
        .parse::<bool>()
        .map_err(|_| BotErrors::WrongQuery)?;

    Ok((index, correct))
}

async fn handle<F, Fut>(bot: &Bot, chat_id: ChatId, callback: F) -> HandlerResult
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = HandlerResult>,
{
    let result = callback().await;

    match result {
        Ok(_) => Ok(()),
        Err(err) => {
            log::error!("Error: {}", err);
            bot.send_message(chat_id, format!("Произошла ошибка при ответе:\n{}\n\nНапишите (нажмите) /start, чтобы продолжить", err))
                .send()
                .await?;
            Ok(())
        }
    }
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
