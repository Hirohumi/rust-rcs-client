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

pub struct MessagingConfigs {
    pub chat_auth: i32,
    pub group_chat_auth: i32,
    pub standalone_msg_auth: i32,

    pub max_ad_hoc_group_size: usize,

    pub conf_fcty_uri: Option<String>,

    pub max_one_to_many_recipients: i32,
    pub one_to_many_selected_technology: i32,

    pub im_session_auto_accept: i32,
    pub im_session_auto_accept_group_chat: i32,
    pub im_session_timer: i32,

    pub max_size_im: usize,

    pub standalone_msg_max_size: usize,
    pub standalone_msg_switch_over_size: usize,

    pub exploder_uri: Option<String>,

    pub chatbot_msg_tech: i32,
}

impl MessagingConfigs {
    pub fn new() -> MessagingConfigs {
        MessagingConfigs {
            chat_auth: 0,
            group_chat_auth: 0,
            standalone_msg_auth: 0,

            max_one_to_many_recipients: 0,
            one_to_many_selected_technology: 0,

            max_ad_hoc_group_size: 0,

            conf_fcty_uri: None,

            im_session_auto_accept: 1,
            im_session_auto_accept_group_chat: 1,
            im_session_timer: 0,

            max_size_im: usize::max_value(),

            standalone_msg_max_size: usize::max_value(),
            standalone_msg_switch_over_size: 1300,

            exploder_uri: None,

            chatbot_msg_tech: 0,
        }
    }

    pub fn update_configuration(&mut self, rcs_app: &RcsApplication) {
        if let Some(services_config) = rcs_app.get_services_config() {
            self.chat_auth = services_config.get_chat_auth();
            self.group_chat_auth = services_config.get_group_chat_auth();
            self.standalone_msg_auth = services_config.get_standalone_msg_auth();
        }

        if let Some(messaging_config) = rcs_app.get_messaging_config() {
            self.max_one_to_many_recipients = messaging_config.get_max_one_to_many_recipients();
            self.one_to_many_selected_technology =
                messaging_config.get_one_to_many_selected_technology();

            if let Some(chat_config) = messaging_config.get_chat_config() {
                self.max_ad_hoc_group_size = chat_config.get_max_ad_hoc_group_size();
                self.conf_fcty_uri = chat_config.get_conf_fcty_uri();
                self.im_session_auto_accept = chat_config.get_im_session_auto_accept();
                self.im_session_auto_accept_group_chat =
                    chat_config.get_im_session_auto_accept_group_chat();
                self.max_size_im = chat_config.get_max_size_im();
            }

            if let Some(standalone_config) = messaging_config.get_standalone_config() {
                self.standalone_msg_max_size = standalone_config.get_max_size();
                self.standalone_msg_switch_over_size = standalone_config.get_switch_over_size();
                self.exploder_uri = standalone_config.get_exploder_uri();
            }

            if let Some(chatbot_config) = messaging_config.get_chat_bot_config() {
                self.chatbot_msg_tech = chatbot_config.get_chatbot_msg_tech();
            }
        }
    }
}
