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

pub struct FileTransferOverHTTPConfigs {
    pub ft_auth: u32,

    pub ft_warn_size: u32,
    pub max_size_file_tr: u32,
    pub ft_aut_accept: u32,

    pub ft_http_cs_uri: Option<String>,
    pub ft_http_dl_uri: Option<String>,
    pub ft_http_cs_user: Option<String>,
    pub ft_http_cs_pwd: Option<String>,
    pub ft_http_fallback: Option<String>,

    pub ft_max_1_to_many_recipients: u32,
}

impl FileTransferOverHTTPConfigs {
    pub fn new() -> FileTransferOverHTTPConfigs {
        FileTransferOverHTTPConfigs {
            ft_auth: 0,

            ft_warn_size: 0,
            max_size_file_tr: 0,
            ft_aut_accept: 0,

            ft_http_cs_uri: None,
            ft_http_dl_uri: None,
            ft_http_cs_user: None,
            ft_http_cs_pwd: None,
            ft_http_fallback: None,

            ft_max_1_to_many_recipients: 0,
        }
    }

    pub fn update_configuration(&mut self, rcs_app: &RcsApplication) {
        if let Some(messaging_config) = rcs_app.get_messaging_config() {
            if let Some(services_config) = rcs_app.get_services_config() {
                self.ft_auth = services_config.get_ft_auth();
            }

            if let Some(file_transfer_config) = messaging_config.get_file_transfer_config() {
                self.ft_warn_size = file_transfer_config.get_ft_warn_size();
                self.max_size_file_tr = file_transfer_config.get_max_size_file_tr();
                self.ft_aut_accept = file_transfer_config.get_ft_aut_accept();

                self.ft_http_cs_uri = file_transfer_config.get_ft_http_cs_uri();
                self.ft_http_dl_uri = file_transfer_config.get_ft_http_dl_uri();
                self.ft_http_cs_user = file_transfer_config.get_ft_http_cs_user();
                self.ft_http_cs_pwd = file_transfer_config.get_ft_http_cs_pwd();
                self.ft_http_fallback = file_transfer_config.get_ft_http_fallback();

                self.ft_max_1_to_many_recipients =
                    file_transfer_config.get_ft_max_1_to_many_recipients();
            }
        }
    }
}
