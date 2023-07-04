// Copyright 2023 宋昊文
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::provisioning::rcs_application::RcsApplication;

pub struct ChatbotConfig {
    pub chatbot_directory: Option<String>,
    pub bot_info_fqdn: Option<String>,
    pub specific_chatbots_lists: Option<String>,
}

impl ChatbotConfig {
    pub fn new() -> ChatbotConfig {
        ChatbotConfig {
            chatbot_directory: None,
            bot_info_fqdn: None,
            specific_chatbots_lists: None,
        }
    }

    pub fn update_configuration(&mut self, rcs_app: &RcsApplication) {
        if let Some(messaging_config) = rcs_app.get_messaging_config() {
            if let Some(chatbot_config) = messaging_config.get_chat_bot_config() {
                self.chatbot_directory = chatbot_config.get_chatbot_directory();
                self.bot_info_fqdn = chatbot_config.get_bot_info_fqdn();
                self.specific_chatbots_lists = chatbot_config.get_specific_chatbots_lists();
            }
        }
    }
}
