use indoc::indoc;
use teloxide::{
    payloads::SendMessageSetters,
    requests::{Request, Requester},
    types::{ChatId, ParseMode},
    Bot,
};

use crate::utils::escape_telegram_symbols;

use super::{
    bot_core::BotContext,
    bot_filter::parse_filter,
    bot_services::{TaskInfoService, UserStateService},
};

#[derive(Debug, thiserror::Error)]
pub(super) enum FilterErrors {}

impl<T: TaskInfoService, U: UserStateService> BotContext<T, U> {
    pub(super) async fn handle_filter(
        &self,
        bot: &Bot,
        command_text: Option<&str>,
        chat_id: ChatId,
    ) -> anyhow::Result<()> {
        match command_text {
            Some(text) => self.change_filter(bot, text, chat_id).await,
            None => self.handle_filter_help(bot, chat_id).await,
        }
    }

    async fn change_filter(&self, bot: &Bot, filter_text: &str, chat_id: ChatId) -> anyhow::Result<()> {
        if filter_text == "-" {
            let mut user_state = self.user_data.get_state(chat_id).await?;
            user_state.filter = None;
            self.user_data.update_state(chat_id, user_state).await?;
            self.user_data.update_tasks(chat_id, &[]).await?;

            self.ask_next_task(bot, chat_id).await?;
            return Ok(());
        }

        let filter = parse_filter(filter_text);
        let task_ids = self.tasks.get_task_ids(Some(&filter)).await?;

        if task_ids.is_empty() {
            bot.send_message(chat_id, "Ничего не найдено по фильтру, попробуйте изменить его")
                .await?;
        } else {
            let mut user_state = self.user_data.get_state(chat_id).await?;
            user_state.filter = Some(filter_text.into());
            self.user_data.update_state(chat_id, user_state).await?;
            self.user_data.update_tasks(chat_id, &[]).await?;

            self.ask_next_task(bot, chat_id).await?;
        }
        Ok(())
    }

    async fn handle_filter_help(&self, bot: &Bot, chat_id: ChatId) -> anyhow::Result<()> {
        let mut message = indoc! {r#"
        Фильтр позволяет выбрать задания по определенным критериям.
        Например, можно выбрать все задания, c падежом genetiv.

        Перечислите через запятую значения, которые нужно оставить. Например, `/filter genetiv, accusative`.

        Чтобы задание удовлетворяло нескольким критериям, перечислите их через точку с запятой. Например, `/filter genetiv, accusative; plural`.

        Чтобы сбросить фильтр, используйте `/filter-reset` или `/filter -`.
        
        Возможные значения:
    "#}.to_owned();

        let filter_info = self.tasks.collect_filter_info().await?;
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
}
