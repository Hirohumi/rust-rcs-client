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

pub struct ChatbotSipUri<'a> {
    pub bot_identifier: &'a str,
    pub bot_publisher_id: &'a str,
    pub bot_platform: &'a str,
    pub bot_platform_domain: &'a str,
}

pub trait AsChatbotSipUri<'a> {
    type Target;
    fn as_chatbot_sip_uri(&'a self) -> Option<Self::Target>;
}

impl<'a> AsChatbotSipUri<'a> for str {
    type Target = ChatbotSipUri<'a>;
    fn as_chatbot_sip_uri(&'a self) -> Option<ChatbotSipUri<'a>> {
        if let Some(idx) = self.find('@') {
            let user_part = &self[..idx];
            let host_part = &self[idx + 1..];

            let (bot_identifier, bot_publisher_id) = if let Some(idx) = user_part.find('.') {
                (&user_part[0..idx], &user_part[idx + 1..])
            } else {
                (user_part, &user_part[0..0])
            };

            let bot_identifier = if bot_identifier.starts_with("sip:") {
                &bot_identifier[4..]
            } else {
                &bot_identifier[..]
            };

            let (bot_platform, bot_platform_domain) = if let Some(idx) = host_part.find('.') {
                (&host_part[0..idx], &host_part[idx + 1..])
            } else {
                (host_part, &host_part[0..0])
            };

            Some(ChatbotSipUri {
                bot_identifier,
                bot_publisher_id,
                bot_platform,
                bot_platform_domain,
            })
        } else {
            None
        }
    }
}
