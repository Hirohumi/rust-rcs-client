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

use rust_rcs_core::cpim::cpim_info::CPIMInfo;

pub struct BotInfo {
    pub maap_traffic_type: Vec<u8>,
}

pub trait GetBotInfo<'a> {
    fn get_bot_related_info(&'a self) -> Option<BotInfo>;
}

impl<'a> GetBotInfo<'a> for CPIMInfo<'a> {
    fn get_bot_related_info(&'a self) -> Option<BotInfo> {
        None
    }
}
