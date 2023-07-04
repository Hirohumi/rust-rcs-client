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

use rust_rcs_core::internet::{
    name_addr::AsNameAddr, parameter::ParameterParser, AsURI, HeaderField,
};

pub enum CPMServiceType {
    OneToOne,
    Group,
    Chatbot,
    System,
}

pub struct CPMContact {
    pub service_uri: Vec<u8>,

    pub service_type: CPMServiceType,

    pub service_support_message_revoke: bool,
    pub service_support_network_fallback: bool,
}

pub trait AsCPMContact {
    type Target;
    fn as_cpm_contact(&self) -> Option<Self::Target>;
}

impl AsCPMContact for HeaderField<'_> {
    type Target = CPMContact;

    fn as_cpm_contact(&self) -> Option<Self::Target> {
        if let Some(name_addr) = self.value.as_name_addresses().first() {
            if let Some(uri_part) = &name_addr.uri_part {
                if let Some(uri) = uri_part.uri.as_standard_uri() {
                    let contact_uri = uri.string_representation_without_query_and_fragment();

                    let mut service_type = CPMServiceType::OneToOne;

                    let mut service_support_message_revoke = false;
                    let mut service_support_network_fallback = false;

                    if let Some(contact) = uri.get_query_value(b"contact") {
                        if let Ok(contact) = std::str::from_utf8(contact) {
                            if let Ok(contact) = urlencoding::decode(contact) {
                                for p in ParameterParser::new(contact.as_bytes(), b';', false) {
                                    if p.name.eq_ignore_ascii_case(b"+g.gsma.rcs.isbot") {
                                        service_type = CPMServiceType::Chatbot;
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    for p in uri_part.get_parameter_iterator() {
                        if p.name.eq_ignore_ascii_case(b"isfocus") {
                            service_type = CPMServiceType::Group;
                        } else if p.name.eq_ignore_ascii_case(b"+g.gsma.rcs.msgrevoke") {
                            service_support_message_revoke = true;
                        } else if p.name.eq_ignore_ascii_case(b"+g.gsma.rcs.msgfallback") {
                            service_support_network_fallback = true;
                        }
                    }

                    return Some(CPMContact {
                        service_uri: contact_uri,
                        service_type,
                        service_support_message_revoke,
                        service_support_network_fallback,
                    });
                }
            }
        }

        None
    }
}
