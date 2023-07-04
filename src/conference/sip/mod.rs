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

use rust_rcs_core::internet::{name_addr::AsNameAddr, AsURI, HeaderField};

pub struct ConferenceV1Contact {
    pub conference_uri: String,
}

pub trait AsConferenceV1Contact {
    type Target;
    fn as_conference_v1_contact(&self) -> Option<Self::Target>;
}

impl AsConferenceV1Contact for HeaderField<'_> {
    type Target = ConferenceV1Contact;

    fn as_conference_v1_contact(&self) -> Option<Self::Target> {
        if let Some(name_addr) = self.value.as_name_addresses().first() {
            if let Some(uri_part) = &name_addr.uri_part {
                if let Some(uri) = uri_part.uri.as_standard_uri() {
                    let contact_uri = uri.string_representation_without_query_and_fragment();

                    for p in uri_part.get_parameter_iterator() {
                        if p.name.eq_ignore_ascii_case(b"isfocus") {
                            if let Some(idx) = contact_uri.iter().position(|c| *c == b'@') {
                                if (&contact_uri[idx + 1..])
                                    .eq_ignore_ascii_case(b"conference.example.com")
                                {
                                    if let Ok(conference_uri) =
                                        std::str::from_utf8(&contact_uri[..idx])
                                    {
                                        return Some(ConferenceV1Contact {
                                            conference_uri: String::from(conference_uri),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }
}
