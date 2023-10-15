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

use super::characteristic::Characteristic;
use super::wap_provisioning_doc::WapProvisioningDoc;

pub struct RcsApplication<'a> {
    root: &'a Characteristic,
}

impl<'a> RcsApplication<'a> {
    pub fn new(e: &Characteristic) -> RcsApplication {
        RcsApplication { root: e }
    }

    pub fn clone_characteristic(&self) -> Characteristic {
        self.root.clone()
    }

    pub fn get_to_app_ref(&self) -> &'a str {
        if let Some(to_app_ref) = self.root.get_parameter("To-AppRef") {
            return to_app_ref;
        }

        "DEFAULT"
    }

    pub fn get_services_config(&self) -> Option<Services> {
        if let Some(c) = self.root.get_child_characteristic("SERVICES") {
            return Some(Services { root: c });
        }

        None
    }

    pub fn get_messaging_config(&self) -> Option<Messaging> {
        if let Some(c) = self.root.get_child_characteristic("MESSAGING") {
            return Some(Messaging { root: c });
        }

        None
    }
}

pub trait GetRcsApplication {
    fn get_rcs_application(&self) -> Option<RcsApplication>;
}

impl<'a> GetRcsApplication for &WapProvisioningDoc {
    fn get_rcs_application(&self) -> Option<RcsApplication> {
        if let Some(rcs_application) = self.get_application_characteristic("ap2002") {
            return Some(RcsApplication {
                root: rcs_application,
            });
        }

        None
    }
}

pub struct Services<'a> {
    root: &'a Characteristic,
}

impl Services<'_> {
    pub fn get_chat_auth(&self) -> i32 {
        if let Some(p) = self.root.get_parameter("ChatAuth") {
            if let Ok(i) = p.parse::<i32>() {
                return i;
            }
        }

        0
    }

    pub fn get_group_chat_auth(&self) -> i32 {
        if let Some(p) = self.root.get_parameter("GroupChatAuth") {
            if let Ok(i) = p.parse::<i32>() {
                return i;
            }
        }

        0
    }

    pub fn get_standalone_msg_auth(&self) -> i32 {
        if let Some(p) = self.root.get_parameter("standaloneMsgAuth") {
            if let Ok(i) = p.parse::<i32>() {
                return i;
            }
        }

        0
    }

    pub fn get_ft_auth(&self) -> u32 {
        if let Some(p) = self.root.get_parameter("ftAuth") {
            if let Ok(i) = p.parse::<u32>() {
                return i;
            }
        }

        0
    }
}

pub struct Messaging<'a> {
    root: &'a Characteristic,
}

impl Messaging<'_> {
    pub fn get_max_one_to_many_recipients(&self) -> i32 {
        if let Some(p) = self.root.get_parameter("max1ToManyRecipients") {
            if let Ok(i) = p.parse::<i32>() {
                return i;
            }
        }

        0
    }

    pub fn get_one_to_many_selected_technology(&self) -> i32 {
        if let Some(p) = self.root.get_parameter("1toManySelectedTech") {
            if let Ok(i) = p.parse::<i32>() {
                return i;
            }
        }

        0
    }

    pub fn get_chat_config(&self) -> Option<Chat> {
        if let Some(c) = self.root.get_child_characteristic("Chat") {
            return Some(Chat { root: c });
        }

        None
    }

    pub fn get_standalone_config(&self) -> Option<StandaloneMsg> {
        if let Some(c) = self.root.get_child_characteristic("StandaloneMsg") {
            return Some(StandaloneMsg { root: c });
        }

        None
    }

    pub fn get_file_transfer_config(&self) -> Option<FileTransfer> {
        if let Some(c) = self.root.get_child_characteristic("FileTransfer") {
            return Some(FileTransfer { root: c });
        }

        None
    }

    pub fn get_chat_bot_config(&self) -> Option<Chatbot> {
        if let Some(c) = self.root.get_child_characteristic("Chatbot") {
            return Some(Chatbot { root: c });
        }

        None
    }
}

pub struct Chat<'a> {
    root: &'a Characteristic,
}

impl Chat<'_> {
    pub fn get_max_ad_hoc_group_size(&self) -> usize {
        if let Some(p) = self.root.get_parameter("max_adhoc_group_size") {
            if let Ok(i) = p.parse::<usize>() {
                return i;
            }
        }

        0
    }

    pub fn get_conf_fcty_uri(&self) -> Option<String> {
        if let Some(p) = self.root.get_parameter("conf-fcty-uri") {
            return Some(String::from(p));
        }

        None
    }

    pub fn get_im_session_auto_accept(&self) -> i32 {
        if let Some(p) = self.root.get_parameter("AutAccept") {
            if let Ok(i) = p.parse::<i32>() {
                return i;
            }
        }

        1
    }

    pub fn get_im_session_auto_accept_group_chat(&self) -> i32 {
        if let Some(p) = self.root.get_parameter("AutAcceptGroupChat") {
            if let Ok(i) = p.parse::<i32>() {
                return i;
            }
        }

        1
    }

    pub fn get_im_session_timer(&self) -> i32 {
        if let Some(p) = self.root.get_parameter("TimerIdle") {
            if let Ok(i) = p.parse::<i32>() {
                return i;
            }
        }

        0
    }

    pub fn get_max_size_im(&self) -> usize {
        if let Some(p) = self.root.get_parameter("MaxSize") {
            if let Ok(i) = p.parse::<usize>() {
                return i;
            }
        }

        usize::max_value()
    }
}

pub struct StandaloneMsg<'a> {
    root: &'a Characteristic,
}

impl StandaloneMsg<'_> {
    pub fn get_max_size(&self) -> usize {
        if let Some(p) = self.root.get_parameter("MaxSize") {
            if let Ok(i) = p.parse::<usize>() {
                return i;
            }
        }

        usize::max_value()
    }

    pub fn get_switch_over_size(&self) -> usize {
        if let Some(p) = self.root.get_parameter("SwitchoverSize") {
            if let Ok(i) = p.parse::<usize>() {
                return i;
            }
        }

        1300
    }

    pub fn get_exploder_uri(&self) -> Option<String> {
        if let Some(p) = self.root.get_parameter("exploder-uri") {
            return Some(String::from(p));
        }

        None
    }
}

pub struct FileTransfer<'a> {
    root: &'a Characteristic,
}

impl FileTransfer<'_> {
    pub fn get_ft_warn_size(&self) -> u32 {
        if let Some(p) = self.root.get_parameter("ftWarnSize") {
            if let Ok(i) = p.parse::<u32>() {
                return i;
            }
        }

        0
    }

    pub fn get_max_size_file_tr(&self) -> u32 {
        if let Some(p) = self.root.get_parameter("MaxSizeFileTr") {
            if let Ok(i) = p.parse::<u32>() {
                return i;
            }
        }

        0
    }

    pub fn get_ft_aut_accept(&self) -> u32 {
        if let Some(p) = self.root.get_parameter("ftAutAccept") {
            if let Ok(i) = p.parse::<u32>() {
                return i;
            }
        }

        0
    }

    pub fn get_ft_http_cs_uri(&self) -> Option<String> {
        if let Some(p) = self.root.get_parameter("ftHTTPCSURI") {
            return Some(String::from(p));
        }

        None
    }

    pub fn get_ft_http_dl_uri(&self) -> Option<String> {
        if let Some(p) = self.root.get_parameter("ftHTTPDLURI") {
            return Some(String::from(p));
        }

        None
    }

    pub fn get_ft_http_cs_user(&self) -> Option<String> {
        if let Some(p) = self.root.get_parameter("ftHTTPCSUser") {
            return Some(String::from(p));
        }

        None
    }

    pub fn get_ft_http_cs_pwd(&self) -> Option<String> {
        if let Some(p) = self.root.get_parameter("ftHTTPCSPwd") {
            return Some(String::from(p));
        }

        None
    }

    pub fn get_ft_http_fallback(&self) -> Option<String> {
        if let Some(p) = self.root.get_parameter("ftHTTPFallback") {
            return Some(String::from(p));
        }

        None
    }

    pub fn get_ft_max_1_to_many_recipients(&self) -> u32 {
        if let Some(p) = self.root.get_parameter("ftMax1ToManyRecipients") {
            if let Ok(i) = p.parse::<u32>() {
                return i;
            }
        }

        0
    }
}

pub struct Chatbot<'a> {
    root: &'a Characteristic,
}

impl Chatbot<'_> {
    pub fn get_chatbot_msg_tech(&self) -> i32 {
        if let Some(p) = self.root.get_parameter("ChatbotMsgTech") {
            if let Ok(i) = p.parse::<i32>() {
                return i;
            }
        }

        0
    }

    pub fn get_chatbot_directory(&self) -> Option<String> {
        if let Some(p) = self.root.get_parameter("ChatbotDirectory") {
            return Some(String::from(p));
        }

        None
    }

    pub fn get_bot_info_fqdn(&self) -> Option<String> {
        if let Some(p) = self.root.get_parameter("BotinfoFQDNRoot") {
            return Some(String::from(p));
        }

        None
    }

    pub fn get_specific_chatbots_lists(&self) -> Option<String> {
        if let Some(p) = self.root.get_parameter("SpecificChatbotsList") {
            return Some(String::from(p));
        }

        None
    }
}
