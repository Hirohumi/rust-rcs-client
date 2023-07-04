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

use std::ffi::CString;

enum MultiConferenceEventInner {
    UserJoin(String),
    UserLeft(String),
    ConferenceEnd,
}

pub struct MultiConferenceEvent {
    inner: MultiConferenceEventInner,
}

impl MultiConferenceEvent {
    pub fn conference_end() -> MultiConferenceEvent {
        MultiConferenceEvent {
            inner: MultiConferenceEventInner::ConferenceEnd,
        }
    }

    pub fn get_event_type(&self) -> u16 {
        match self.inner {
            MultiConferenceEventInner::UserJoin(_) => 0,
            MultiConferenceEventInner::UserLeft(_) => 1,
            MultiConferenceEventInner::ConferenceEnd => 2,
        }
    }

    pub fn get_user_joined(&self) -> Result<CString, ()> {
        if let MultiConferenceEventInner::UserJoin(user) = &self.inner {
            if let Ok(user) = CString::new(user.as_str()) {
                return Ok(user);
            }
        }

        Err(())
    }

    pub fn get_user_left(&self) -> Result<CString, ()> {
        if let MultiConferenceEventInner::UserLeft(user) = &self.inner {
            if let Ok(user) = CString::new(user.as_str()) {
                return Ok(user);
            }
        }

        Err(())
    }
}

pub struct MultiConferenceSubscriber {}
